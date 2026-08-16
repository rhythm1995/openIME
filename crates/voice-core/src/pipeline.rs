//! 端到端 pipeline：编排"采集 → 转写 → 插入 → 落库"。
//!
//! 设计：`Pipeline` 持有四个 trait 对象（Arc），`record_once` 跑一次完整录音会话：
//! 1. provider.connect 建立会话；
//! 2. 起 reader 任务消费 deltas：partial 经 `on_partial` 回调（UI 显示），
//!    final 经 inserter 插入前台 App + 落库为 utterance；
//! 3. 主循环：audio.next_frame → session.feed；
//! 4. 录音停止（audio 返回 None 或外部 cancel）→ session.finish → 等 reader 结束。
//!
//! P1：按 [`SessionIntent`] 分流（听写 / 翻译 / QA 录音）；听写 L0 后做前缀角色检测
//! （R5），命中直连 cloud/local，未命中走 [`crate::polish::PolishRouter`]。
//! 插入走四态 `insert_ex`（R7），`InsertOpts` 由薄壳组装传入。
//!
//! 完全用 mock 可测：FakeAudioSource + FakeAsrProvider + RecordingInserter + InMemoryStore。

use std::sync::Arc;

use futures::StreamExt;
use uuid::Uuid;

use crate::insert::InsertOpts;
use crate::polish::{
    detect_prefix_role, lang_display_name, starts_with_assistant, LlmClient, TranslateRequest,
};
use crate::store::RoleKind;
use crate::traits::{
    AsrProvider, AudioSource, HistoryStore, PolishMode, PolishRequest, SessionSummary,
    TextInserter, TextPolishProvider, TranscriptKind, UtteranceRecord,
};
use crate::{Error, InsertOutcome};

/// pipeline 的依赖。全部以 Arc<dyn> 注入，便于 mock 与替换。
pub struct PipelineDeps {
    pub provider: Arc<dyn AsrProvider>,
    pub inserter: Arc<dyn TextInserter>,
    pub store: Arc<dyn HistoryStore>,
    /// 无前缀 Light/Heavy 润色路由；None 则直通原文。
    pub polish: Option<Arc<dyn TextPolishProvider>>,
    /// P1：云端 LLM（翻译 / 前缀角色 / QA）。与 polish 路由分开注入。
    pub cloud: Option<Arc<dyn LlmClient>>,
    /// P1：本地 GGUF（仅 `provider=local` 的前缀角色直连 + 译前 Light）。
    pub local: Option<Arc<dyn TextPolishProvider>>,
    /// 本地三件套：本地专翻（`LlmClient::translate_text`）。
    pub dedicated: Option<Arc<dyn LlmClient>>,
    /// 本地三件套：兼译句柄（润色模型兼做翻译；通常与 `local` 同实例）。
    pub local_llm: Option<Arc<dyn LlmClient>>,
}

/// P1：一次录音会话的意图（快捷键来源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionIntent {
    /// 听写（录音键）：L0 → 前缀角色 → 润色路由。
    #[default]
    Dictate,
    /// 翻译（翻译键）：说源语言，出目标语言。不走前缀 / 风格包 / Router。
    Translate,
    /// QA 录音（QA 窗可见时按录音键）：只转写不插入。
    Qa,
}

/// 润色/处理阶段的可展示警告（薄壳据此发 HUD 文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolishWarn {
    TranslateFailed,
    RoleLlmFailed,
    RoleNoBackend,
}

/// 单条 final 的处理结果：上屏文本 + 警告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolishOutcome {
    pub text: String,
    pub warning: Option<PolishWarn>,
}

/// `insert_finals_with_polish` 的单条结果：文本 + 插入四态 + 警告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalInsertResult {
    pub text: String,
    pub outcome: InsertOutcome,
    pub warning: Option<PolishWarn>,
}

/// 润色上下文（录音结束插入前使用）。
#[derive(Debug, Clone, Default)]
pub struct PolishContext {
    pub enabled: bool,
    pub mode: PolishMode,
    pub style_prompt: Option<String>,
    pub hotwords: Vec<String>,
    pub timeout_ms: u32,
    /// R2:润色取消标志；`Some` 且被置 true 时，apply_polish 在当前 await 点尽快返回 L0 结果。
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// P1：会话意图。
    pub intent: SessionIntent,
    /// P1：前缀角色开关（开 → 听写不流式上屏，由薄壳保证）。
    pub prefix_roles_enabled: bool,
    /// R5：助手名称——「助手名+别名」组合触发前缀角色（空 = 不触发）。
    pub assistant_name: String,
    /// P1：风格包全表（前缀检测 + 角色 prompt）。
    pub style_packs: Vec<crate::StylePack>,
    /// P1：翻译目标语言短码（如 "en"）。
    pub translate_target_lang: String,
    /// P1：「先润色再翻译」：云端哨兵合成；本地 = Light 源语纠错再译（两步）。
    pub translate_with_polish: bool,
    /// 本地三件套：翻译路由策略（PreferCloud 默认 / PreferLocal）。
    pub translate_policy: crate::config::TranslatePolicy,
    /// 本地三件套：弱机兼译开关（专翻不可用时用润色模型兼做翻译）。
    pub translate_use_llm_fallback: bool,
    /// 本地三件套：源语言短码（ASR `local_language`；`auto` 由脚本粗分）。
    pub source_lang: String,
}

impl PolishContext {
    /// R4/R5 共用：翻译 / 角色 LLM 超时 = max(polish_timeout_ms, 8000)。
    pub fn llm_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis((self.timeout_ms.max(8000)).max(100) as u64)
    }
}

/// partial 增量回调（UI 用）。一期可忽略返回。
pub type PartialCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// 一次录音会话的结果汇总。
#[derive(Debug, Clone, Default)]
pub struct SessionResult {
    pub session_id: String,
    pub utterances: Vec<String>,
}

/// 一次录音会话需要的上下文：引擎/provider/model 元信息（来自当前配置）。
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub engine: String,
    pub provider: String,
    pub model: String,
}

/// pipeline 编排器。`audio` 在每次 record_once 时由调用方提供（便于复用/替换）。
pub struct Pipeline {
    deps: PipelineDeps,
}

impl Pipeline {
    pub fn new(deps: PipelineDeps) -> Self {
        Self { deps }
    }

    /// 跑一次完整录音会话。
    ///
    /// - `audio`：音频源（已 start 或将由本方法 start）。
    /// - `cfg`：provider 配置（传给 connect）。
    /// - `meta`：会话元信息（存入 session）。
    /// - `on_partial`：partial 回调；传 `None` 则忽略 partial。
    /// - `stop_flag`：外部停止标志；置 true 后本方法在当前帧后停止喂音频并 finish。
    ///
    /// 返回会话结果（session_id + 各 final 文本）。文本由内部 `insert_finals` 插入前台。
    pub async fn record_once(
        &self,
        audio: Box<dyn AudioSource>,
        cfg: &crate::ProviderConfig,
        meta: SessionMeta,
        on_partial: Option<PartialCallback>,
        stop_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> crate::Result<SessionResult> {
        let result = self
            .record_and_collect(
                audio,
                cfg,
                meta,
                on_partial,
                stop_flag,
                false,
                &InsertOpts::default(),
            )
            .await?;
        self.insert_finals(&result.session_id, &result.utterances)
            .await?;
        Ok(result)
    }

    /// 只录音并收集各 final 文本，**不插入**前台 App。
    ///
    /// 用于需要先恢复前台焦点（如 macOS 上 overlay 抢焦点）再插入的场景：
    /// 调用方拿到结果后应先激活目标 app，再调 [`Self::insert_finals`]。
    ///
    /// `streaming_insert`：true 时 partial/final 经 diff_prefix 增量上屏（C1）；
    /// chunk 失败（FR-7.10）→ 停止继续逐字，final 时对 diff 粘贴一次。
    #[allow(clippy::too_many_arguments)]
    pub async fn record_and_collect(
        &self,
        mut audio: Box<dyn AudioSource>,
        cfg: &crate::ProviderConfig,
        meta: SessionMeta,
        on_partial: Option<PartialCallback>,
        stop_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
        streaming_insert: bool,
        insert_opts: &InsertOpts,
    ) -> crate::Result<SessionResult> {
        let session_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // 建立会话记录（先建，便于即使中途失败也留痕）。
        self.deps
            .store
            .create_session(&SessionSummary {
                id: session_id.clone(),
                title: meta.model.clone(),
                started_at: now,
                ended_at: None,
                engine: meta.engine.clone(),
                provider: meta.provider.clone(),
                model: meta.model.clone(),
            })
            .await?;

        // 连接 ASR。
        let mut asr = self.deps.provider.connect(cfg).await?;
        // 先取出 deltas 流（'static），reader 任务持有它，不持有 asr。
        let deltas = asr.deltas();

        // reader：消费 deltas，收集 final，partial 走回调。
        // C1 streaming_insert=true 时，partial/final 经 diff_prefix 增量上屏（Unicode 安全）。
        let partial_cb = on_partial;
        let inserter = self.deps.inserter.clone();
        let inserted: Arc<std::sync::Mutex<String>> =
            Arc::new(std::sync::Mutex::new(String::new()));
        let streaming = streaming_insert;
        let opts = insert_opts.clone();
        let reader = tokio::spawn(async move {
            let mut finals: Vec<String> = Vec::new();
            let mut deltas = deltas;
            // R7（FR-7.10）：chunk 失败后停止逐字，记录失败时已上屏的文本，final 时贴一次。
            let mut broken: Option<String> = None;
            while let Some(item) = deltas.next().await {
                match item {
                    Ok(d) => match d.kind {
                        TranscriptKind::Partial => {
                            if let Some(cb) = &partial_cb {
                                cb(&d.text);
                            }
                            if streaming && broken.is_none() {
                                let delta = {
                                    let s = inserted.lock().unwrap();
                                    let delta = crate::insert::diff_prefix(&s, &d.text).to_string();
                                    delta
                                };
                                if !delta.is_empty() {
                                    let outcome = inserter.insert_ex(&delta, &opts).await;
                                    let mut s = inserted.lock().unwrap();
                                    match outcome {
                                        InsertOutcome::Typed | InsertOutcome::Pasted => {
                                            *s = d.text.clone();
                                        }
                                        _ => {
                                            broken = Some(s.clone());
                                        }
                                    }
                                }
                            }
                        }
                        TranscriptKind::Final => {
                            if streaming {
                                match broken.take() {
                                    Some(before) => {
                                        // 停止逐字后：对剩余差异粘贴一次。
                                        let delta = crate::insert::diff_prefix(&before, &d.text)
                                            .to_string();
                                        if !delta.is_empty() {
                                            let mut paste_opts = opts.clone();
                                            paste_opts.strategy =
                                                crate::config::InsertStrategy::Paste;
                                            let _ = inserter.insert_ex(&delta, &paste_opts).await;
                                        }
                                    }
                                    None => {
                                        let delta = {
                                            let mut s = inserted.lock().unwrap();
                                            let delta =
                                                crate::insert::diff_prefix(&s, &d.text).to_string();
                                            s.clear(); // 句末：下一句从零开始
                                            delta
                                        };
                                        if !delta.is_empty() {
                                            let _ = inserter.insert_ex(&delta, &opts).await;
                                        }
                                    }
                                }
                            }
                            finals.push(d.text.clone());
                        }
                    },
                    Err(_) => break,
                }
            }
            finals
        });

        // 主循环：喂音频。检查外部停止标志。
        audio.start().await?;
        loop {
            if let Some(flag) = &stop_flag {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
            }
            match audio.next_frame().await {
                Some(Ok(frame)) => asr.feed(&frame).await?,
                Some(Err(_)) => break,
                None => break,
            }
        }
        audio.stop().await?;
        asr.finish().await?;

        let finals = reader
            .await
            .map_err(|e| Error::Insert(format!("reader 任务 panic: {e}")))?;

        Ok(SessionResult {
            session_id,
            utterances: finals,
        })
    }

    /// 把已收集的 final 文本插入前台 App 并落库。
    ///
    /// 调用方负责：需要在插入前确保目标窗口（前台 App）已获得焦点。
    pub async fn insert_finals(&self, session_id: &str, finals: &[String]) -> crate::Result<()> {
        self.insert_finals_with_polish(
            session_id,
            finals,
            &PolishContext::default(),
            &InsertOpts::default(),
        )
        .await
        .map(|_| ())
    }

    /// 插入前按意图处理（L0 → 前缀角色 / 翻译 / 润色路由），走四态 `insert_ex`。
    /// 返回每条 final 的（文本、插入结果、警告）。
    pub async fn insert_finals_with_polish(
        &self,
        session_id: &str,
        finals: &[String],
        ctx: &PolishContext,
        insert_opts: &InsertOpts,
    ) -> crate::Result<Vec<FinalInsertResult>> {
        // ASR 有时会连续推两条相同 final；先去重再润色/上屏，避免「同一句输入两次」。
        let finals = crate::polish::dedupe_consecutive_finals(finals);
        let mut last_inserted = String::new();
        let mut results = Vec::new();
        for (seq, text) in finals.iter().enumerate() {
            let polished = self.apply_polish(text, ctx).await;
            if polished.text.is_empty() {
                continue;
            }
            // 上屏级再挡一层：连续两条润色结果相同则只插一次。
            if polished.text == last_inserted {
                tracing::debug!("跳过与上一条相同的上屏文本");
                continue;
            }
            let outcome = self
                .deps
                .inserter
                .insert_ex(&polished.text, insert_opts)
                .await;
            last_inserted = polished.text.clone();
            self.deps
                .store
                .save_utterance(&UtteranceRecord {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.to_string(),
                    seq: seq as u32,
                    // 落库保存实际上屏文本（处理后）。
                    final_text: polished.text.clone(),
                    audio_path: None,
                    created_at: chrono::Utc::now(),
                })
                .await?;
            results.push(FinalInsertResult {
                text: polished.text,
                outcome,
                warning: polished.warning,
            });
        }
        Ok(results)
    }

    /// C1：流式模式专用——finals 已在录音期间逐字上屏，只去重+落库，不重复插入。
    pub async fn persist_finals(&self, session_id: &str, finals: &[String]) -> crate::Result<()> {
        let finals = crate::polish::dedupe_consecutive_finals(finals);
        let mut last = String::new();
        for (seq, text) in finals.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() || text == last {
                continue;
            }
            self.deps
                .store
                .save_utterance(&UtteranceRecord {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.to_string(),
                    seq: seq as u32,
                    final_text: text.to_string(),
                    audio_path: None,
                    created_at: chrono::Utc::now(),
                })
                .await?;
            last = text.to_string();
        }
        Ok(())
    }

    /// 单条文本的意图处理：返回上屏文本与可选警告。
    async fn apply_polish(&self, text: &str, ctx: &PolishContext) -> PolishOutcome {
        if text.trim().is_empty() {
            return PolishOutcome {
                text: text.to_string(),
                warning: None,
            };
        }
        // ── L0 规则层：总是先过一遍（即使总体润色关闭，也做最小清理）；不阻断。
        let l0 = crate::polish::correct_l0(text, &ctx.hotwords);
        if l0.text.trim().is_empty() {
            return PolishOutcome {
                text: l0.text,
                warning: None,
            };
        }
        tracing::debug!(
            "L0 规则层：had_correction={} truncation={} 原='{}' 纠后='{}'",
            l0.had_correction,
            l0.truncation_flag,
            text,
            l0.text
        );

        match ctx.intent {
            // ── R4 翻译：不走前缀 / 风格包 / Router；失败回退 L0 原文。
            SessionIntent::Translate => self.apply_translate(&l0.text, ctx).await,
            // ── 听写：L0 后前缀角色（R5），未命中走润色路由。
            SessionIntent::Dictate => {
                if ctx.prefix_roles_enabled {
                    if let Some((pack, rest)) =
                        detect_prefix_role(&l0.text, &ctx.assistant_name, &ctx.style_packs)
                    {
                        return self.apply_prefix_role(pack, &rest, ctx).await;
                    }
                    // 句首是助手名但组合未命中（如「小友你好」）：交给润色模型会把
                    // 助手名当正文改坏，跳过润色直出 L0 原文。
                    if starts_with_assistant(&l0.text, &ctx.assistant_name) {
                        tracing::debug!("句首是助手名但未命中角色组合，跳过润色直出：{}", l0.text);
                        return PolishOutcome {
                            text: l0.text,
                            warning: None,
                        };
                    }
                }
                self.apply_routed_polish(&l0.text, ctx).await
            }
            // ── QA 录音不进插入路径；防御性 L0 直出。
            SessionIntent::Qa => PolishOutcome {
                text: l0.text,
                warning: None,
            },
        }
    }

    /// R4：翻译（先润色再翻译可选）。失败 → L0 原文 + TranslateFailed（FR-4.3/4.4）。
    ///
    /// 本地三件套：走 [`crate::polish::TranslateRouter`]（云 / 专翻 / 兼译）。
    /// `translate_with_polish`：云端第一跳保留哨兵合成；本地 = Light 源语纠错再译
    /// （两步，禁哨兵）；Light 失败跳过，仍译 L0。
    async fn apply_translate(&self, text: &str, ctx: &PolishContext) -> PolishOutcome {
        let router = self.translate_router(ctx);
        if router.is_empty() {
            return PolishOutcome {
                text: text.to_string(),
                warning: Some(PolishWarn::TranslateFailed),
            };
        }
        let target = lang_display_name(&ctx.translate_target_lang).to_string();
        let build_req = |t: &str| TranslateRequest {
            text: t.to_string(),
            target_lang: target.clone(),
            source_lang: ctx.source_lang.clone(),
            timeout: ctx.llm_timeout(),
            max_tokens: 1024,
        };

        // 译前 Light：仅本地路径需要（云端哨兵自身会润色）。
        let src = if ctx.translate_with_polish && !router.first_hop_is_cloud() {
            match self.light_pre_polish(text, ctx).await {
                Some(polished) => polished,
                None => text.to_string(), // Light 失败跳过，仍译 L0。
            }
        } else {
            text.to_string()
        };

        if ctx.translate_with_polish && router.first_hop_is_cloud() {
            // 云端哨兵合成（仅云端第一跳）。
            match router.polish_and_translate(&build_req(&src)).await {
                Ok(pt) if !pt.translation.trim().is_empty() => {
                    tracing::info!("润色+翻译成功：{} 字译文", pt.translation.chars().count());
                    return PolishOutcome {
                        text: pt.translation.trim().to_string(),
                        warning: None,
                    };
                }
                Ok(_) => tracing::warn!("润色+翻译输出无效，回退纯翻译"),
                Err(e) => tracing::warn!("润色+翻译失败，回退纯翻译：{e}"),
            }
            // FR-4.4：哨兵失败 → 完整路由纯翻译；再失败 → L0 原文。
        }

        match router.translate(&build_req(&src)).await {
            Ok(t) if !t.trim().is_empty() => PolishOutcome {
                text: t.trim().to_string(),
                warning: None,
            },
            Err(e) => {
                tracing::warn!("翻译失败，回退 L0 原文：{e}");
                PolishOutcome {
                    text: text.to_string(),
                    warning: Some(PolishWarn::TranslateFailed),
                }
            }
            Ok(_) => PolishOutcome {
                text: text.to_string(),
                warning: Some(PolishWarn::TranslateFailed),
            },
        }
    }

    /// 按上下文 + deps 组装翻译路由（无状态，构造廉价）。
    fn translate_router(&self, ctx: &PolishContext) -> crate::polish::TranslateRouter {
        crate::polish::TranslateRouter {
            policy: ctx.translate_policy,
            cloud: self.deps.cloud.clone(),
            dedicated: self.deps.dedicated.clone(),
            llm_fallback: self.deps.local_llm.clone(),
            use_llm_fallback: ctx.translate_use_llm_fallback,
        }
    }

    /// 译前 Light：仅走本地 GGUF（`deps.local`），禁止 style_prompt，超时用 polish_timeout_ms。
    async fn light_pre_polish(&self, text: &str, ctx: &PolishContext) -> Option<String> {
        let local = self.deps.local.as_ref()?;
        let req = PolishRequest {
            text: text.to_string(),
            mode: PolishMode::Light,
            style_prompt: None,
            hotwords: ctx.hotwords.clone(),
            timeout: std::time::Duration::from_millis(ctx.timeout_ms.max(100) as u64),
            max_tokens: None,
        };
        match local.polish(req).await {
            Ok(r) if !r.text.trim().is_empty() && !r.provider.contains("passthrough") => {
                Some(r.text.trim().to_string())
            }
            _ => None,
        }
    }

    /// R5：前缀角色——Translate 走翻译，其它按包 provider 直连（禁止 PolishRouter）。
    /// 失败 → 去前缀原文 + RoleLlmFailed / RoleNoBackend。
    async fn apply_prefix_role(
        &self,
        pack: &crate::StylePack,
        rest: &str,
        ctx: &PolishContext,
    ) -> PolishOutcome {
        tracing::debug!("前缀角色命中：{}（{}）", pack.name, pack.id);
        if pack.role_kind == RoleKind::Translate {
            // 本地三件套：翻译角色与 R4 共用 TranslateRouter（不加听写风格包）。
            let router = self.translate_router(ctx);
            if router.is_empty() {
                return PolishOutcome {
                    text: rest.to_string(),
                    warning: Some(PolishWarn::RoleNoBackend),
                };
            }
            let target = lang_display_name(&ctx.translate_target_lang).to_string();
            let req = TranslateRequest {
                text: rest.to_string(),
                target_lang: target,
                source_lang: ctx.source_lang.clone(),
                timeout: ctx.llm_timeout(),
                max_tokens: 1024,
            };
            return match router.translate(&req).await {
                Ok(t) if !t.trim().is_empty() => PolishOutcome {
                    text: t.trim().to_string(),
                    warning: None,
                },
                _ => PolishOutcome {
                    text: rest.to_string(),
                    warning: Some(PolishWarn::RoleLlmFailed),
                },
            };
        }

        // 普通指令角色的后端选择：
        // - provider 未指定（默认）→ 跟随 AI 润色：本地 GGUF 优先，无本地走云端。
        // - provider=local → 仅本地；provider=cloud → 仅云端（缺失即 RoleNoBackend）。
        let provider = pack.provider.as_deref().map(str::trim);
        let explicit_local = provider
            .filter(|p| !p.is_empty())
            .map(|p| p.eq_ignore_ascii_case("local"))
            .unwrap_or(false);
        let explicit_cloud = provider
            .filter(|p| !p.is_empty())
            .map(|p| p.eq_ignore_ascii_case("cloud"))
            .unwrap_or(false);
        let req = PolishRequest {
            text: rest.to_string(),
            mode: PolishMode::Heavy,
            style_prompt: Some(pack.system_prompt.clone()),
            hotwords: ctx.hotwords.clone(),
            timeout: ctx.llm_timeout(),
            max_tokens: Some(1024),
        };
        // cloud（LlmClient::polish）与 local（TextPolishProvider::polish）分开调用，
        // 避免 trait 对象间转换。
        let result = if explicit_cloud {
            match &self.deps.cloud {
                Some(cloud) => cloud.polish(req).await,
                None => {
                    return PolishOutcome {
                        text: rest.to_string(),
                        warning: Some(PolishWarn::RoleNoBackend),
                    }
                }
            }
        } else if explicit_local {
            match &self.deps.local {
                Some(local) => local.polish(req).await,
                None => {
                    return PolishOutcome {
                        text: rest.to_string(),
                        warning: Some(PolishWarn::RoleNoBackend),
                    }
                }
            }
        } else {
            // 默认跟随润色（PreferLocal 语义）：本地可用走本地，否则云端兜底。
            match (&self.deps.local, &self.deps.cloud) {
                (Some(local), _) => local.polish(req).await,
                (None, Some(cloud)) => cloud.polish(req).await,
                (None, None) => {
                    return PolishOutcome {
                        text: rest.to_string(),
                        warning: Some(PolishWarn::RoleNoBackend),
                    }
                }
            }
        };
        match result {
            Ok(r) if !r.text.trim().is_empty() => PolishOutcome {
                text: r.text.trim().to_string(),
                warning: None,
            },
            Ok(_) => PolishOutcome {
                text: rest.to_string(),
                warning: Some(PolishWarn::RoleLlmFailed),
            },
            Err(e) => {
                tracing::warn!("角色 LLM 失败，插入去前缀原文：{e}");
                PolishOutcome {
                    text: rest.to_string(),
                    warning: Some(PolishWarn::RoleLlmFailed),
                }
            }
        }
    }

    /// 无前缀命中时的路由润色（F1 行为）：Off → L0 直出；Light/Heavy → Router + 全局包。
    async fn apply_routed_polish(&self, text: &str, ctx: &PolishContext) -> PolishOutcome {
        // 若总体润色关闭 / 无 provider / 模式 Off → L0 直出。
        if !ctx.enabled || ctx.mode == PolishMode::Off {
            return PolishOutcome {
                text: text.to_string(),
                warning: None,
            };
        }
        let Some(polish) = &self.deps.polish else {
            return PolishOutcome {
                text: text.to_string(),
                warning: None,
            };
        };

        // ── L2 gating：≤8 字跳过 LLM（过度纠正 + 延迟不值得；调研 6.3）。
        if text.trim().chars().count() <= 8 {
            tracing::debug!("L2 跳过：≤8 字，L0 直出");
            return PolishOutcome {
                text: text.to_string(),
                warning: None,
            };
        }

        // ── L2 LLM 纯校对（失败→ L0 回退，不阻断上屏）。
        let req = PolishRequest {
            text: text.to_string(),
            mode: ctx.mode,
            style_prompt: ctx.style_prompt.clone(),
            hotwords: ctx.hotwords.clone(),
            timeout: std::time::Duration::from_millis(ctx.timeout_ms.max(100) as u64),
            max_tokens: None,
        };
        // R2:支持 ESC 取消——润色进行中若 cancel 标志被置 true，尽快返回 L0 结果。
        let polish_fut = polish.polish(req);
        let result = match &ctx.cancel {
            Some(flag) => tokio::select! {
                r = polish_fut => Some(r),
                _ = wait_cancel(flag.clone()) => {
                    tracing::info!("润色被用户取消（ESC），使用 L0 结果");
                    None
                }
            },
            None => Some(polish_fut.await),
        };
        match result {
            Some(Ok(r)) => {
                if r.text.trim().is_empty() {
                    PolishOutcome {
                        text: text.to_string(),
                        warning: None,
                    }
                } else {
                    let cleaned = crate::polish::sanitize_polish_output(text, &r.text);
                    if cleaned != r.text.trim() {
                        tracing::info!(
                            "润色输出已清洗（防重复）：provider={} raw_len={} clean_len={}",
                            r.provider,
                            r.text.len(),
                            cleaned.len()
                        );
                    }
                    PolishOutcome {
                        text: cleaned,
                        warning: None,
                    }
                }
            }
            Some(Err(e)) => {
                tracing::warn!("润色失败，使用 L0 结果：{e}");
                PolishOutcome {
                    text: text.to_string(),
                    warning: None,
                }
            }
            None => PolishOutcome {
                text: text.to_string(),
                warning: None,
            },
        }
    }
}

/// R2:轮询取消标志（30ms），用于 `tokio::select!` 与润色 future 竞速。
async fn wait_cancel(flag: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InsertStrategy;
    use crate::traits::{AsrSession, AudioFormat, AudioFrame, PolishResponse, TranscriptDelta};
    use crate::ProviderConfig;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex as StdMutex;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    // ---- fakes ----

    struct FakeAudio {
        frames: VecDeque<AudioFrame>,
    }
    impl FakeAudio {
        fn new(n: usize) -> Self {
            Self {
                frames: (0..n)
                    .map(|_| AudioFrame::new(AudioFormat::PCM_16K_MONO_S16LE, vec![0u8; 640]))
                    .collect(),
            }
        }
    }
    #[async_trait]
    impl AudioSource for FakeAudio {
        async fn start(&mut self) -> crate::Result<()> {
            Ok(())
        }
        async fn next_frame(&mut self) -> Option<crate::Result<AudioFrame>> {
            self.frames.pop_front().map(Ok)
        }
        async fn stop(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    struct FakeSession {
        rx: StdMutex<Option<tokio::sync::mpsc::UnboundedReceiver<crate::Result<TranscriptDelta>>>>,
        tx: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<crate::Result<TranscriptDelta>>>>,
    }
    impl AsrSession for FakeSession {
        fn feed(
            &mut self,
            _frame: &AudioFrame,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }
        fn finish(
            &mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
        {
            let tx = self.tx.lock().unwrap().take();
            Box::pin(async move {
                if let Some(tx) = tx {
                    let _ = tx.send(Ok(TranscriptDelta::partial("你好", 0)));
                    let _ = tx.send(Ok(TranscriptDelta::final_("你好世界", 0)));
                    // tx drop：接收流自然结束。
                }
                Ok(())
            })
        }
        fn deltas(
            &mut self,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = crate::Result<TranscriptDelta>> + Send>>
        {
            let rx = self.rx.lock().unwrap().take().unwrap();
            Box::pin(UnboundedReceiverStream::new(rx))
        }
    }

    struct FakeProvider;
    #[async_trait]
    impl AsrProvider for FakeProvider {
        async fn connect(&self, _cfg: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Ok(Box::new(FakeSession {
                rx: StdMutex::new(Some(rx)),
                tx: StdMutex::new(Some(tx)),
            }))
        }
    }

    #[derive(Default)]
    struct RecInserter {
        out: StdMutex<String>,
    }
    #[async_trait]
    impl TextInserter for RecInserter {
        async fn insert(&self, text: &str) -> crate::Result<()> {
            self.out.lock().unwrap().push_str(text);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemStore {
        sessions: StdMutex<Vec<SessionSummary>>,
        utterances: StdMutex<Vec<UtteranceRecord>>,
    }
    #[async_trait]
    impl HistoryStore for MemStore {
        async fn create_session(&self, s: &SessionSummary) -> crate::Result<()> {
            self.sessions.lock().unwrap().push(s.clone());
            Ok(())
        }
        async fn save_utterance(&self, u: &UtteranceRecord) -> crate::Result<()> {
            self.utterances.lock().unwrap().push(u.clone());
            Ok(())
        }
        async fn list_sessions(&self) -> crate::Result<Vec<SessionSummary>> {
            Ok(self.sessions.lock().unwrap().clone())
        }
        async fn list_utterances(&self, _sid: &str) -> crate::Result<Vec<UtteranceRecord>> {
            Ok(self.utterances.lock().unwrap().clone())
        }
        async fn delete_session(&self, sid: &str) -> crate::Result<()> {
            self.sessions.lock().unwrap().retain(|s| s.id != sid);
            Ok(())
        }
    }

    fn deps() -> (PipelineDeps, Arc<RecInserter>, Arc<MemStore>) {
        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: None,
            cloud: None,
            local: None,
            dedicated: None,
            local_llm: None,
        };
        (deps, ins, store)
    }

    #[tokio::test]
    async fn pipeline_inserts_final_and_stores() {
        let (deps, ins, store) = deps();
        let pipe = Pipeline::new(deps);

        let partial_count = Arc::new(StdMutex::new(0u32));
        let pc = partial_count.clone();
        let on_partial: PartialCallback = Arc::new(move |_| {
            *pc.lock().unwrap() += 1;
        });

        let cfg = ProviderConfig {
            kind: crate::ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "test".into(),
            vocabulary_id: None,
            language: None,
        };
        let meta = SessionMeta {
            engine: "local".into(),
            provider: "fake".into(),
            model: "test".into(),
        };

        let result = pipe
            .record_once(
                Box::new(FakeAudio::new(3)),
                &cfg,
                meta,
                Some(on_partial),
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.utterances, vec!["你好世界"]);
        assert_eq!(*ins.out.lock().unwrap(), "你好世界");
        assert_eq!(*partial_count.lock().unwrap(), 1);
        assert_eq!(store.sessions.lock().unwrap().len(), 1);
        assert_eq!(store.utterances.lock().unwrap().len(), 1);
        assert_eq!(store.utterances.lock().unwrap()[0].final_text, "你好世界");
    }

    #[tokio::test]
    async fn pipeline_creates_session_even_if_no_finals() {
        // 一个不发任何 delta 的 provider。
        struct EmptySession;
        impl AsrSession for EmptySession {
            fn feed(
                &mut self,
                _f: &AudioFrame,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
            fn finish(
                &mut self,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
            fn deltas(
                &mut self,
            ) -> std::pin::Pin<Box<dyn futures::Stream<Item = crate::Result<TranscriptDelta>> + Send>>
            {
                Box::pin(futures::stream::empty())
            }
        }
        struct EmptyProvider;
        #[async_trait]
        impl AsrProvider for EmptyProvider {
            async fn connect(&self, _c: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
                Ok(Box::new(EmptySession))
            }
        }

        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let pipe = Pipeline::new(PipelineDeps {
            provider: Arc::new(EmptyProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: None,
            cloud: None,
            local: None,
            dedicated: None,
            local_llm: None,
        });

        let cfg = ProviderConfig {
            kind: crate::ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "test".into(),
            vocabulary_id: None,
            language: None,
        };
        let meta = SessionMeta {
            engine: "local".into(),
            provider: "fake".into(),
            model: "test".into(),
        };
        let r = pipe
            .record_once(Box::new(FakeAudio::new(1)), &cfg, meta, None, None)
            .await
            .unwrap();
        assert!(r.utterances.is_empty());
        assert_eq!(store.sessions.lock().unwrap().len(), 1);
        assert!(ins.out.lock().unwrap().is_empty());
    }

    // ── L0 / L2 / 回退 集成测试（TDD）──────────────────────────

    enum MockBehavior {
        Ok(String),
        Empty,
        Err,
        /// provider="passthrough"（模拟润色 Off/空文本早退），text 可与输入不同，
        /// 用于断言 pipeline 把 passthrough 结果视为「未润色」。
        Passthrough(String),
    }

    struct MockPolish {
        calls: Arc<AtomicU32>,
        behavior: MockBehavior,
    }
    impl MockPolish {
        fn new(b: MockBehavior) -> Self {
            Self {
                calls: Arc::new(AtomicU32::new(0)),
                behavior: b,
            }
        }
    }
    #[async_trait]
    impl TextPolishProvider for MockPolish {
        async fn polish(&self, _req: PolishRequest) -> crate::Result<PolishResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                MockBehavior::Ok(t) => Ok(PolishResponse {
                    text: t.clone(),
                    provider: "mock".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Empty => Ok(PolishResponse {
                    text: String::new(),
                    provider: "mock".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Err => Err(crate::Error::Provider("mock fail".into())),
                MockBehavior::Passthrough(t) => Ok(PolishResponse {
                    text: t.clone(),
                    provider: "passthrough".into(),
                    latency_ms: 0,
                }),
            }
        }
    }

    fn deps_with_polish(
        polish: Arc<dyn TextPolishProvider>,
    ) -> (PipelineDeps, Arc<RecInserter>, Arc<MemStore>) {
        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: Some(polish),
            cloud: None,
            local: None,
            dedicated: None,
            local_llm: None,
        };
        (deps, ins, store)
    }

    fn ctx_enabled(mode: PolishMode) -> PolishContext {
        PolishContext {
            enabled: true,
            mode,
            style_prompt: None,
            hotwords: vec![],
            timeout_ms: 1000,
            cancel: None,
            intent: SessionIntent::Dictate,
            prefix_roles_enabled: false,
            assistant_name: "小友".into(),
            style_packs: vec![],
            translate_target_lang: "en".into(),
            translate_with_polish: false,
            translate_policy: crate::config::TranslatePolicy::PreferCloud,
            translate_use_llm_fallback: false,
            source_lang: "auto".into(),
        }
    }

    fn opts_default() -> InsertOpts {
        InsertOpts {
            strategy: InsertStrategy::Auto,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn l0_cleanup_runs_even_when_polish_disabled() {
        // 总开关关闭 / 无 provider：L0 规则层仍生效（去填充词 + 补句号）。
        let (deps, ins, _store) = deps(); // polish: None
        let pipe = Pipeline::new(deps);
        let ctx = PolishContext {
            enabled: false,
            ..Default::default()
        };
        pipe.insert_finals_with_polish("s1", &["嗯那个今天天气不错".into()], &ctx, &opts_default())
            .await
            .unwrap();
        let out = ins.out.lock().unwrap().clone();
        assert!(
            out.contains("今天天气不错"),
            "L0 应去掉首部填充词，得到 {out}"
        );
        assert!(!out.ends_with('。'), "B4：单句输入不应补句号，得到 {out}");
    }

    #[tokio::test]
    async fn l2_skipped_when_l0_result_short() {
        // L0 结果 ≤8 字 → 不调用 LLM（调研 6.3：过度纠正 + 延迟不值得）。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("不该出现".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["你好".into()],
            &ctx_enabled(PolishMode::Light),
            &opts_default(),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "你好");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "短句(≤8字)不应调用 L2");
    }

    #[tokio::test]
    async fn l2_used_for_long_text_and_inserts_corrected() {
        // 长句 → L2 校对，返回纠正文本 → 上屏纠正后结果。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok(
            "我们下午在会议室见面吧".into(),
        )));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
            &opts_default(),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会议室见面吧");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn l2_error_falls_back_to_l0() {
        // L2 失败 → 不阻断，回退 L0 结果上屏。
        let mock = Arc::new(MockPolish::new(MockBehavior::Err));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
            &opts_default(),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会试室见面吧");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn l2_empty_output_falls_back_to_l0() {
        // L2 返回空串 → 视同无效，回退 L0。
        let mock = Arc::new(MockPolish::new(MockBehavior::Empty));
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
            &opts_default(),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会试室见面吧");
    }

    #[tokio::test]
    async fn empty_final_is_skipped() {
        // 空 final 不应触发 polish，也不上屏。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("x".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["".into()],
            &ctx_enabled(PolishMode::Light),
            &opts_default(),
        )
        .await
        .unwrap();
        assert!(ins.out.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // ── P1：翻译 / 前缀角色 mock 测试（A4.4b / A5.x）────────────

    struct MockCloud {
        translate_calls: Arc<AtomicU32>,
        polish_translate_calls: Arc<AtomicU32>,
        behavior: MockBehavior,
    }
    impl MockCloud {
        fn new(b: MockBehavior) -> Self {
            Self {
                translate_calls: Arc::new(AtomicU32::new(0)),
                polish_translate_calls: Arc::new(AtomicU32::new(0)),
                behavior: b,
            }
        }
    }
    #[async_trait]
    impl LlmClient for MockCloud {
        async fn polish(&self, _req: PolishRequest) -> crate::Result<PolishResponse> {
            match &self.behavior {
                MockBehavior::Ok(t) => Ok(PolishResponse {
                    text: t.clone(),
                    provider: "mock-cloud".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Empty => Ok(PolishResponse {
                    text: String::new(),
                    provider: "mock-cloud".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Passthrough(t) => Ok(PolishResponse {
                    text: t.clone(),
                    provider: "passthrough".into(),
                    latency_ms: 0,
                }),
                MockBehavior::Err => Err(crate::Error::Provider("mock cloud fail".into())),
            }
        }
        async fn translate_text(&self, _req: TranslateRequest) -> crate::Result<String> {
            self.translate_calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                MockBehavior::Ok(t) => Ok(t.clone()),
                MockBehavior::Empty => Ok(String::new()),
                MockBehavior::Passthrough(t) => Ok(t.clone()),
                MockBehavior::Err => Err(crate::Error::Provider("mock translate fail".into())),
            }
        }
        async fn polish_and_translate(
            &self,
            req: TranslateRequest,
        ) -> crate::Result<crate::polish::PolishTranslate> {
            self.polish_translate_calls.fetch_add(1, Ordering::SeqCst);
            if req.text.contains("哨兵失败") {
                return Ok(crate::polish::PolishTranslate {
                    polished: "坏输出".into(),
                    translation: String::new(),
                });
            }
            Ok(crate::polish::PolishTranslate {
                polished: format!("润色:{}", req.text),
                translation: format!("T({})", req.text),
            })
        }
        async fn chat_stream(&self, _req: crate::polish::ChatRequest) -> crate::Result<String> {
            Ok("QA 回答".into())
        }
    }

    fn role_pack(id: &str, prefix: &str, kind: RoleKind) -> crate::StylePack {
        crate::StylePack {
            id: id.into(),
            name: id.into(),
            system_prompt: "角色指令".into(),
            is_builtin: true,
            ord: 0,
            match_prefix: Some(prefix.into()),
            provider: None,
            model: None,
            role_kind: kind,
            output_mode: crate::store::OutputMode::Insert,
        }
    }

    fn ctx_with_cloud(cloud: Arc<dyn LlmClient>) -> (PipelineDeps, Arc<RecInserter>) {
        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store,
            polish: None,
            cloud: Some(cloud),
            local: None,
            dedicated: None,
            local_llm: None,
        };
        (deps, ins)
    }

    #[tokio::test]
    async fn translate_intent_uses_translate_text() {
        // A4.4b：mock LlmClient 返回固定译文 → 插入 mock 文本。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok(
            "We have a meeting tomorrow.".into(),
        )));
        let calls = mock.translate_calls.clone();
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "We have a meeting tomorrow.");
        assert_eq!(results[0].warning, None);
        assert_eq!(*ins.out.lock().unwrap(), "We have a meeting tomorrow.");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn translate_failure_falls_back_to_l0_with_warning() {
        // FR-4.3：失败回退 L0 原文（不丢字）+ TranslateFailed。
        let mock = Arc::new(MockCloud::new(MockBehavior::Err));
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::TranslateFailed));
        assert_eq!(*ins.out.lock().unwrap(), "明天开会");
    }

    #[tokio::test]
    async fn translate_with_polish_uses_sentinel_call() {
        // A4.4b：polish+translate 走合成调用一次。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("unused".into())));
        let pt_calls = mock.polish_translate_calls.clone();
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "T(明天开会)");
        assert_eq!(pt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*ins.out.lock().unwrap(), "T(明天开会)");
    }

    #[tokio::test]
    async fn translate_with_polish_bad_sentinels_retries_pure_translate() {
        // FR-4.4：合成解析失败 → 再走纯 translate_text。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok(
            "We have a meeting.".into(),
        )));
        let translate_calls = mock.translate_calls.clone();
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        let results = pipe
            .insert_finals_with_polish("s1", &["哨兵失败 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        // 合成调用把空译文解析成「失败」→ 回退纯翻译。
        assert_eq!(results[0].text, "We have a meeting.");
        assert_eq!(results[0].warning, None);
        assert_eq!(translate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*ins.out.lock().unwrap(), "We have a meeting.");
    }

    #[tokio::test]
    async fn translate_without_cloud_falls_back_with_warning() {
        let (deps, ins, _store) = deps();
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::TranslateFailed));
        assert_eq!(*ins.out.lock().unwrap(), "明天开会");
    }

    // ── 本地三件套：翻译路由（T9）────────────

    /// 记录收到的文本 + 固定输出的翻译后端。
    struct RecordingTranslate {
        out: String,
        calls: Arc<AtomicU32>,
        last_text: StdMutex<Option<String>>,
    }
    impl RecordingTranslate {
        fn new(out: &str) -> Self {
            Self {
                out: out.into(),
                calls: Arc::new(AtomicU32::new(0)),
                last_text: StdMutex::new(None),
            }
        }
    }
    #[async_trait]
    impl LlmClient for RecordingTranslate {
        async fn translate_text(&self, req: TranslateRequest) -> crate::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_text.lock().unwrap() = Some(req.text.clone());
            Ok(self.out.clone())
        }
        async fn polish(&self, _req: PolishRequest) -> crate::Result<PolishResponse> {
            Err(crate::Error::Provider("no polish".into()))
        }
        async fn polish_and_translate(
            &self,
            _req: TranslateRequest,
        ) -> crate::Result<crate::polish::PolishTranslate> {
            Err(crate::Error::Provider("本地禁止哨兵".into()))
        }
        async fn chat_stream(&self, _req: crate::polish::ChatRequest) -> crate::Result<String> {
            Err(crate::Error::Provider("no chat".into()))
        }
    }

    fn deps_local_translate(
        dedicated: Option<Arc<dyn LlmClient>>,
        local_llm: Option<Arc<dyn LlmClient>>,
        local_polish: Option<Arc<dyn TextPolishProvider>>,
    ) -> (PipelineDeps, Arc<RecInserter>) {
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: None,
            local: local_polish,
            dedicated,
            local_llm,
        };
        (deps, ins)
    }

    #[tokio::test]
    async fn translate_uses_dedicated_when_no_cloud() {
        // 无云 + 专翻已装 → 本地专翻直出。
        let dedi = Arc::new(RecordingTranslate::new("Local translation."));
        let (deps, ins) = deps_local_translate(Some(dedi.clone()), None, None);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Local translation.");
        assert_eq!(results[0].warning, None);
        assert_eq!(*ins.out.lock().unwrap(), "Local translation.");
        assert_eq!(dedi.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn translate_with_polish_local_does_light_then_translate() {
        // 本地两步：先 Light（deps.local），再专翻；禁哨兵。
        let dedi = Arc::new(RecordingTranslate::new("Local translation."));
        let local = Arc::new(MockPolish::new(MockBehavior::Ok("【L】明天开会".into())));
        let local_calls = local.calls.clone();
        let (deps, ins) = deps_local_translate(Some(dedi.clone()), None, Some(local));
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        ctx.translate_policy = crate::config::TranslatePolicy::PreferLocal;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Local translation.");
        assert_eq!(local_calls.load(Ordering::SeqCst), 1);
        // 专翻收到的是 Light 之后的文本（两步，不是 L0）。
        assert_eq!(
            dedi.last_text.lock().unwrap().as_deref(),
            Some("【L】明天开会")
        );
        assert_eq!(*ins.out.lock().unwrap(), "Local translation.");
    }

    #[tokio::test]
    async fn translate_light_failure_still_translates_l0() {
        // Light 失败 → 跳过，仍译 L0（不 abort）。
        let dedi = Arc::new(RecordingTranslate::new("Local translation."));
        let local = Arc::new(MockPolish::new(MockBehavior::Err));
        let local_calls = local.calls.clone();
        let (deps, ins) = deps_local_translate(Some(dedi.clone()), None, Some(local));
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        ctx.translate_policy = crate::config::TranslatePolicy::PreferLocal;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Local translation.");
        assert_eq!(local_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dedi.last_text.lock().unwrap().as_deref(), Some("明天开会"));
        assert_eq!(*ins.out.lock().unwrap(), "Local translation.");
    }

    #[tokio::test]
    async fn translate_uses_llm_fallback_when_dedicated_absent() {
        // 兼译：专翻 None + fallback 开启 → 润色模型兼做翻译。
        let fb = Arc::new(RecordingTranslate::new("Fallback translation."));
        let (deps, ins) = deps_local_translate(None, Some(fb.clone()), None);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_use_llm_fallback = true;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Fallback translation.");
        assert_eq!(fb.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*ins.out.lock().unwrap(), "Fallback translation.");
    }

    #[tokio::test]
    async fn translate_fallback_switch_off_skips_fallback() {
        // fallback 句柄存在但开关关 → 无后端 → TranslateFailed。
        let fb = Arc::new(RecordingTranslate::new("不应被调"));
        let (deps, ins) = deps_local_translate(None, Some(fb.clone()), None);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::TranslateFailed));
        assert_eq!(fb.calls.load(Ordering::SeqCst), 0);
        assert_eq!(*ins.out.lock().unwrap(), "明天开会");
    }

    #[tokio::test]
    async fn translate_prefer_local_skips_cloud_sentinel() {
        // PreferLocal 且专翻可用 → 第一跳不是云 → 禁用哨兵，云不被调用。
        let cloud = Arc::new(MockCloud::new(MockBehavior::Ok("cloud".into())));
        let pt_calls = cloud.polish_translate_calls.clone();
        let dedi = Arc::new(RecordingTranslate::new("Local translation."));
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: None,
            dedicated: Some(dedi),
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        ctx.translate_policy = crate::config::TranslatePolicy::PreferLocal;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Local translation.");
        assert_eq!(
            pt_calls.load(Ordering::SeqCst),
            0,
            "本地第一跳不应走云端哨兵"
        );
        assert_eq!(*ins.out.lock().unwrap(), "Local translation.");
    }

    #[tokio::test]
    async fn translate_sentinel_failure_falls_back_to_full_route() {
        // PreferCloud + 润色翻译：云端哨兵输出无效（坏哨兵）→ 回退**完整路由**
        // （云纯翻译 Err → 专翻成功）。旧实现哨兵失败只重试云；路由化后应能落到专翻。
        let cloud = Arc::new(MockCloud::new(MockBehavior::Err));
        let pt_calls = cloud.polish_translate_calls.clone();
        let t_calls = cloud.translate_calls.clone();
        let dedi = Arc::new(RecordingTranslate::new("本地专翻译文"));
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: None,
            dedicated: Some(dedi.clone()),
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true; // 云第一跳 → 走哨兵
        let results = pipe
            .insert_finals_with_polish("s1", &["哨兵失败 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(pt_calls.load(Ordering::SeqCst), 1, "云端第一跳应先试哨兵");
        assert_eq!(
            t_calls.load(Ordering::SeqCst),
            1,
            "哨兵坏输出后应重试云纯翻译"
        );
        assert_eq!(
            dedi.calls.load(Ordering::SeqCst),
            1,
            "云纯翻译失败后应落专翻"
        );
        assert_eq!(results[0].text, "本地专翻译文");
        assert_eq!(results[0].warning, None);
        assert_eq!(*ins.out.lock().unwrap(), "本地专翻译文");
    }

    #[tokio::test]
    async fn translate_light_passthrough_result_treated_as_unpolished() {
        // 译前 Light 返回 passthrough（润色 Off/空文本早退）→ 视为未润色，
        // 专翻收到 L0 原文，而不是 passthrough 的文本。
        let dedi = Arc::new(RecordingTranslate::new("Local translation."));
        let local = Arc::new(MockPolish::new(MockBehavior::Passthrough(
            "passthrough 输出不应进入翻译".into(),
        )));
        let (deps, ins) = deps_local_translate(Some(dedi.clone()), None, Some(local));
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.intent = SessionIntent::Translate;
        ctx.translate_with_polish = true;
        ctx.translate_policy = crate::config::TranslatePolicy::PreferLocal;
        let results = pipe
            .insert_finals_with_polish("s1", &["明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "Local translation.");
        assert_eq!(
            dedi.last_text.lock().unwrap().as_deref(),
            Some("明天开会"),
            "passthrough 结果应被过滤，专翻收到 L0 原文"
        );
        assert_eq!(*ins.out.lock().unwrap(), "Local translation.");
    }

    // ── A5.x：前缀角色 ──

    #[tokio::test]
    async fn prefix_role_inserts_mock_output_without_prefix() {
        // A5.1b：mock 返回固定邮件体 → 插入 mock，前缀已剥。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("正式邮件正文".into())));
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "正式邮件正文");
        assert_eq!(*ins.out.lock().unwrap(), "正式邮件正文");
    }

    #[tokio::test]
    async fn translate_role_uses_translate_text_not_polish() {
        // A5.2b：role_kind=Translate + mock translate → 走 translate_text。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("你好".into())));
        let translate_calls = mock.translate_calls.clone();
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.translate_target_lang = "zh".into();
        ctx.style_packs = vec![role_pack("translate", "翻译", RoleKind::Translate)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友翻译: hello".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "你好");
        assert_eq!(
            translate_calls.load(Ordering::SeqCst),
            1,
            "应走 translate_text"
        );
        assert_eq!(*ins.out.lock().unwrap(), "你好");
    }

    #[tokio::test]
    async fn fullwidth_colon_same_detection_result() {
        // A5.3：「翻译：hello」与「翻译: hello」同一检测结果。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("你好".into())));
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.translate_target_lang = "zh".into();
        ctx.style_packs = vec![role_pack("translate", "翻译", RoleKind::Translate)];
        for t in ["小友翻译: hello", "小友翻译：hello"] {
            let results = pipe
                .insert_finals_with_polish("s1", &[t.into()], &ctx, &opts_default())
                .await
                .unwrap();
            assert_eq!(results[0].text, "你好", "输入 {t}");
        }
        assert_eq!(*ins.out.lock().unwrap(), "你好你好");
    }

    #[tokio::test]
    async fn no_prefix_does_not_call_role() {
        // A5.4：无前缀 → 不调角色（polish_mode=Off → L0 直出）。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("邮件正文".into())));
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["明天三点开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天三点开会");
        assert_eq!(*ins.out.lock().unwrap(), "明天三点开会");
    }

    #[tokio::test]
    async fn prefix_role_runs_even_when_polish_off() {
        // A5.5：polish_mode=Off + 前缀 → 仍调角色 cloud。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("正式邮件正文".into())));
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.enabled = false;
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "正式邮件正文");
        assert_eq!(*ins.out.lock().unwrap(), "正式邮件正文");
    }

    #[tokio::test]
    async fn assistant_prefix_without_role_skips_polish() {
        // 句首是助手名但组合未命中（「小友你好」）：角色不触发，
        // 跳过润色直出 L0，防止润色模型把助手名当正文改坏。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("润色改坏输出".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Light);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("translate", "翻译", RoleKind::Translate)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友你好呀".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "小友你好呀");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "句首助手名未命中组合时不应调润色"
        );
        assert_eq!(*ins.out.lock().unwrap(), "小友你好呀");
    }

    #[tokio::test]
    async fn bare_alias_text_polishes_normally() {
        // 无助手名的正文（「翻译我明天早上就要走了」）按普通文本正常润色。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("润色结果".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Light);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("translate", "翻译", RoleKind::Translate)];
        let results = pipe
            .insert_finals_with_polish(
                "s1",
                &["翻译我明天早上就要走了这句话".into()],
                &ctx,
                &opts_default(),
            )
            .await
            .unwrap();
        assert_eq!(results[0].text, "润色结果");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*ins.out.lock().unwrap(), "润色结果");
    }

    #[tokio::test]
    async fn assistant_guard_inactive_when_roles_disabled() {
        // 对照：前缀角色开关关闭 → 守卫不生效，句首「小友」仍按普通文本走润色
        //（文本 >8 字确保 L2 短句跳过规则不拦截）。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("润色结果".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Light);
        ctx.prefix_roles_enabled = false;
        ctx.style_packs = vec![role_pack("translate", "翻译", RoleKind::Translate)];
        let results = pipe
            .insert_finals_with_polish(
                "s1",
                &["小友你好呀今天天气怎么样".into()],
                &ctx,
                &opts_default(),
            )
            .await
            .unwrap();
        assert_eq!(results[0].text, "润色结果");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*ins.out.lock().unwrap(), "润色结果");
    }

    #[tokio::test]
    async fn prefix_role_without_backend_inserts_stripped_text_with_warning() {
        // A5.6：无 backend → 插入去前缀原文 + warning。
        let (deps, ins, _store) = deps(); // cloud: None
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::RoleNoBackend));
        assert_eq!(*ins.out.lock().unwrap(), "明天开会");
    }

    #[tokio::test]
    async fn prefix_disabled_treats_prefix_as_plain_text() {
        // A5.7：关前缀开关 → 「邮件: …」当普通文本。
        let mock = Arc::new(MockCloud::new(MockBehavior::Ok("不应出现".into())));
        let translate_calls = mock.translate_calls.clone();
        let (deps, ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = false;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "小友邮件: 明天开会");
        assert_eq!(translate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(*ins.out.lock().unwrap(), "小友邮件: 明天开会");
    }

    #[tokio::test]
    async fn prefix_role_llm_failure_inserts_stripped_text_with_warning() {
        // 角色 LLM 失败 → 去前缀原文 + RoleLlmFailed。
        let mock = Arc::new(MockCloud::new(MockBehavior::Err));
        let (deps, _ins) = ctx_with_cloud(mock);
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::RoleLlmFailed));
    }

    #[tokio::test]
    async fn local_provider_role_uses_local_backend() {
        // provider=local → 走 local handle 而非 cloud。
        let local = Arc::new(MockPolish::new(MockBehavior::Ok("本地角色输出".into())));
        let local_calls = local.calls.clone();
        let cloud = Arc::new(MockCloud::new(MockBehavior::Ok("云端不应被调".into())));
        let cloud_calls = cloud.translate_calls.clone();
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: Some(local),
            dedicated: None,
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        let mut pack = role_pack("mail", "邮件", RoleKind::Default);
        pack.provider = Some("local".into());
        ctx.style_packs = vec![pack];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "本地角色输出");
        assert_eq!(local_calls.load(Ordering::SeqCst), 1);
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 0);
        assert_eq!(*ins.out.lock().unwrap(), "本地角色输出");
    }

    #[tokio::test]
    async fn default_provider_role_follows_polish_prefers_local() {
        // provider 未指定（默认）→ 跟随 AI 润色：本地可用优先本地，云端不参与。
        let local = Arc::new(MockPolish::new(MockBehavior::Ok("本地角色输出".into())));
        let local_calls = local.calls.clone();
        let cloud = Arc::new(MockCloud::new(MockBehavior::Ok("云端不应被调".into())));
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: Some(local),
            dedicated: None,
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        ctx.style_packs = vec![role_pack("mail", "邮件", RoleKind::Default)]; // provider=None
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "本地角色输出");
        assert_eq!(
            local_calls.load(Ordering::SeqCst),
            1,
            "默认应本地优先（跟随润色）"
        );
        assert_eq!(*ins.out.lock().unwrap(), "本地角色输出");
    }

    #[tokio::test]
    async fn explicit_cloud_role_ignores_local() {
        // provider=cloud → 即使本地可用也只走云端。
        let local = Arc::new(MockPolish::new(MockBehavior::Ok("本地不应被调".into())));
        let local_calls = local.calls.clone();
        let cloud = Arc::new(MockCloud::new(MockBehavior::Ok("云端角色输出".into())));
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: Some(local),
            dedicated: None,
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        let mut pack = role_pack("mail", "邮件", RoleKind::Default);
        pack.provider = Some("cloud".into());
        ctx.style_packs = vec![pack];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "云端角色输出");
        assert_eq!(
            local_calls.load(Ordering::SeqCst),
            0,
            "显式 cloud 不应走本地"
        );
        assert_eq!(*ins.out.lock().unwrap(), "云端角色输出");
    }

    #[tokio::test]
    async fn local_provider_role_without_gguf_is_no_backend() {
        // provider=local 但未装 GGUF → RoleNoBackend。
        let cloud = Arc::new(MockCloud::new(MockBehavior::Ok("云端不应被调".into())));
        let ins = Arc::new(RecInserter::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: Some(cloud),
            local: None,
            dedicated: None,
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let mut ctx = ctx_enabled(PolishMode::Off);
        ctx.prefix_roles_enabled = true;
        let mut pack = role_pack("mail", "邮件", RoleKind::Default);
        pack.provider = Some("local".into());
        ctx.style_packs = vec![pack];
        let results = pipe
            .insert_finals_with_polish("s1", &["小友邮件: 明天开会".into()], &ctx, &opts_default())
            .await
            .unwrap();
        assert_eq!(results[0].text, "明天开会");
        assert_eq!(results[0].warning, Some(PolishWarn::RoleNoBackend));
    }

    // ── R7：四态插入 ──

    struct OutcomeInserter {
        outcomes: StdMutex<Vec<(String, InsertOutcome)>>,
    }
    #[async_trait]
    impl TextInserter for OutcomeInserter {
        async fn insert(&self, text: &str) -> crate::Result<()> {
            self.outcomes
                .lock()
                .unwrap()
                .push((text.to_string(), InsertOutcome::Typed));
            Ok(())
        }
        async fn insert_ex(&self, text: &str, _opts: &InsertOpts) -> InsertOutcome {
            let outcome = if text.starts_with('败') {
                InsertOutcome::CopiedFallback
            } else {
                InsertOutcome::Typed
            };
            self.outcomes
                .lock()
                .unwrap()
                .push((text.to_string(), outcome));
            outcome
        }
    }

    #[tokio::test]
    async fn insert_finals_reports_outcomes() {
        // A7.6：插入结果四态被返回给薄壳做 HUD 映射。
        let ins = Arc::new(OutcomeInserter {
            outcomes: StdMutex::new(vec![]),
        });
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: Arc::new(MemStore::default()),
            polish: None,
            cloud: None,
            local: None,
            dedicated: None,
            local_llm: None,
        };
        let pipe = Pipeline::new(deps);
        let ctx = PolishContext {
            enabled: false,
            ..Default::default()
        };
        let results = pipe
            .insert_finals_with_polish(
                "s1",
                &["好的文本".into(), "败了".into()],
                &ctx,
                &opts_default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].outcome, InsertOutcome::Typed);
        assert_eq!(results[1].outcome, InsertOutcome::CopiedFallback);
        let calls = ins.outcomes.lock().unwrap();
        assert!(calls.iter().any(|(t, _)| t == "好的文本"));
    }
}
