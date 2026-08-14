//! 应用全局状态：DB + 配置 + pipeline（懒初始化，避免启动期 enigo 等副作用导致 abort）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::AppHandle;
use tokio::sync::RwLock;
use voice_core::pipeline::{Pipeline, PipelineDeps, PolishContext, SessionIntent};
use voice_core::{
    AppConfig, BailianChatPolish, CloudPolishProvider, Error, HistoryStore, LlmClient,
    LocalGgufPolish, PolishMode, PolishPolicy, PolishRouter, PolishRouterConfig, RoutingProvider,
    SqliteStore, TextInserter, TextPolishProvider,
};

use crate::insert_fallback::CompositeInserter;

pub const CONFIG_KEY: &str = "app_config";

/// 进程级共享状态。
pub struct AppState {
    pub store: Arc<SqliteStore>,
    pub config: Arc<RwLock<AppConfig>>,
    pub sherpa_root: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// pipeline 懒初始化：首次录音时建立（含 EnigoInserter，可能在无辅助功能权限时失败）。
    pipeline: RwLock<Option<Arc<Pipeline>>>,
    /// 当前是否正在录音（前端/overlay 查询用）。
    pub recording: Arc<RwLock<bool>>,
    /// 录音启动原子 guard：CAS 抢占式置位，防止两次 trigger_toggle 在
    /// `recording` RwLock 写入前并发各启一个 pipeline（导致「说一遍录入两遍」）。
    /// 启动时 false→true；pipeline 结束后 false。与 `recording` 语义一致但原子。
    pub recording_guard: Arc<AtomicBool>,
    /// 录音停止标志：toggle 再次调用时置 true，录音循环看到后停止。
    pub stop_flag: Arc<AtomicBool>,
    /// 本地模型下载中（防并发重复下载）。
    pub model_downloading: Arc<AtomicBool>,
    /// R2:润色取消标志；ESC 触发后置 true，apply_polish 看到→尽快返回 L0。
    pub polish_cancel: Arc<AtomicBool>,
    /// P2 R12：文件转录进行中 guard（CAS 防并发启动）。
    pub transcribe_guard: Arc<AtomicBool>,
    /// P2 R12：文件转录取消标志（cancel_transcribe 置 true；命令入口 swap false 再跑）。
    pub transcribe_cancel: Arc<AtomicBool>,
    /// P2 R9：录音中止标志（仅防御；R9 主路径不置位）。置位 → 不上屏、不 QA 提问。
    pub abort_flag: Arc<AtomicBool>,
    /// P1：快捷键先写意图，toggle_recording 抢到 guard 后 take（见 p1-design 分支表）。
    pub pending_intent: Mutex<SessionIntent>,
    /// R7：组合插入器（Type-then-Paste）。与 pipeline 共享同一实例。
    inserter: OnceLock<Arc<CompositeInserter>>,
    /// Tauri 句柄（剪贴板主线程调度 / QA 插入用）。
    app: AppHandle,
}

impl AppState {
    pub fn new(
        app: AppHandle,
        store: SqliteStore,
        sherpa_root: Option<(std::path::PathBuf, std::path::PathBuf)>,
    ) -> Result<Self, Error> {
        let mut config = load_config(&store)?.unwrap_or_default();
        // 迁移：旧版本快捷键硬编码 Alt+Shift+D、设置项保存后并不生效，
        // 该值不代表用户主动选择，统一迁到新默认 Fn。
        if config.hotkey == "Alt+Shift+D" {
            config.hotkey = "Fn".to_string();
        }
        // 迁移：旧版本默认 bailian + 空 base_url，新版本默认 sherpa。
        // 如果用户从未配置云端地址，把 provider 切到 sherpa（本地引擎）。
        if let Some(p) = config.providers.get_mut(config.active_provider) {
            if p.kind == voice_core::ProviderKind::Bailian && p.base_url.trim().is_empty() {
                p.kind = voice_core::ProviderKind::Sherpa;
                p.model = "sherpa-onnx-streaming-paraformer-bilingual-zh-en".to_string();
            }
        }
        let _ = store.seed_builtin_style_packs_if_empty();
        // R5：内置前缀角色包按 id 补缺失（不清用户改动）。
        let _ = store.seed_builtin_prefix_packs_if_missing();
        let store_arc: Arc<SqliteStore> = Arc::new(store);
        Ok(Self {
            store: store_arc,
            config: Arc::new(RwLock::new(config)),
            sherpa_root,
            pipeline: RwLock::new(None),
            recording: Arc::new(RwLock::new(false)),
            recording_guard: Arc::new(AtomicBool::new(false)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            model_downloading: Arc::new(AtomicBool::new(false)),
            polish_cancel: Arc::new(AtomicBool::new(false)),
            transcribe_guard: Arc::new(AtomicBool::new(false)),
            transcribe_cancel: Arc::new(AtomicBool::new(false)),
            abort_flag: Arc::new(AtomicBool::new(false)),
            pending_intent: Mutex::new(SessionIntent::Dictate),
            inserter: OnceLock::new(),
            app,
        })
    }

    /// 本地模型根目录（下载/安装本地引擎用）。
    pub fn model_root(&self) -> Option<std::path::PathBuf> {
        self.sherpa_root.as_ref().map(|(root, _)| root.clone())
    }

    /// R7：组合插入器（懒初始化，与 pipeline 共享）。
    pub fn composite_inserter(&self) -> Result<Arc<CompositeInserter>, Error> {
        if let Some(i) = self.inserter.get() {
            return Ok(i.clone());
        }
        let i = Arc::new(CompositeInserter::new(self.app.clone())?);
        let _ = self.inserter.set(i.clone());
        Ok(i)
    }

    /// 取得 pipeline；首次调用时构造。enigo 初始化失败则返回错误（不会 abort 启动）。
    pub async fn pipeline(&self) -> Result<Arc<Pipeline>, Error> {
        {
            let guard = self.pipeline.read().await;
            if let Some(p) = guard.as_ref() {
                return Ok(p.clone());
            }
        }
        let mut guard = self.pipeline.write().await;
        // double-check
        if let Some(p) = guard.as_ref() {
            return Ok(p.clone());
        }
        let provider: Arc<dyn voice_core::AsrProvider> = Arc::new(RoutingProvider {
            sherpa_root: self.sherpa_root.clone(),
        });
        let inserter: Arc<dyn TextInserter> = self.composite_inserter()?;
        let polish = Some(self.build_polish_router().await);
        // P1：分开的 cloud（LlmClient）与 local（GGUF）句柄。
        let cloud = self.cloud_llm().await;
        let local = self.local_polish().await;
        let deps = PipelineDeps {
            provider,
            inserter,
            store: self.store.clone() as Arc<dyn HistoryStore>,
            polish,
            cloud,
            local,
        };
        let p = Arc::new(Pipeline::new(deps));
        *guard = Some(p.clone());
        Ok(p)
    }

    /// 配置变更后丢弃 pipeline，使润色/引擎设置在下次录音生效。
    pub async fn invalidate_pipeline(&self) {
        let mut guard = self.pipeline.write().await;
        *guard = None;
    }

    /// 按当前配置构造润色路由（本地 GGUF + 可选云端 chat）。
    pub async fn build_polish_router(&self) -> Arc<dyn TextPolishProvider> {
        let cfg = self.config.read().await.clone();
        let local = self.local_polish_from(&cfg);
        let cloud = self
            .cloud_polish_from(&cfg)
            .map(|c| Arc::new(c) as Arc<dyn TextPolishProvider>);

        let polish_on = cfg.polish_mode != PolishMode::Off;
        Arc::new(PolishRouter {
            cfg: PolishRouterConfig {
                policy: if polish_on {
                    PolishPolicy::PreferLocal
                } else {
                    PolishPolicy::Off
                },
                enabled: polish_on,
            },
            local,
            cloud,
        })
    }

    /// P1：云端 LLM 句柄（翻译 / 前缀角色 / QA）。与润色路由分开。
    pub async fn cloud_llm(&self) -> Option<Arc<dyn LlmClient>> {
        let cfg = self.config.read().await.clone();
        self.cloud_polish_from(&cfg)
            .map(|c| Arc::new(c) as Arc<dyn LlmClient>)
    }

    /// P1：本地 GGUF 句柄（provider=local 的前缀角色）。
    pub async fn local_polish(&self) -> Option<Arc<dyn TextPolishProvider>> {
        let cfg = self.config.read().await.clone();
        self.local_polish_from(&cfg)
    }

    fn local_polish_from(&self, _cfg: &AppConfig) -> Option<Arc<dyn TextPolishProvider>> {
        let model_root = self.model_root();
        model_root.map(|root| {
            let path = voice_core::polish_model_path(&root);
            Arc::new(LocalGgufPolish::new(path)) as Arc<dyn TextPolishProvider>
        })
    }

    /// 云端润色：优先用独立配置（polish_cloud_endpoint/api_key/protocol），
    /// 否则回退从 bailian provider 取 key + 默认 base。
    fn cloud_polish_from(&self, cfg: &AppConfig) -> Option<CloudPolishProvider> {
        if !cfg.polish_cloud_endpoint.trim().is_empty()
            && !cfg.polish_cloud_api_key.trim().is_empty()
        {
            let base = cfg.polish_cloud_endpoint.trim().to_string();
            let key = cfg.polish_cloud_api_key.trim().to_string();
            let model = if cfg.polish_cloud_model.trim().is_empty() {
                "qwen-turbo".into()
            } else {
                cfg.polish_cloud_model.clone()
            };
            Some(BailianChatPolish::new_with_protocol(
                key,
                base,
                model,
                cfg.polish_cloud_protocol,
            ))
        } else {
            // 回退：从 bailian provider 取 key + 默认 base。
            cfg.providers
                .iter()
                .find(|p| {
                    p.kind == voice_core::ProviderKind::Bailian && !p.api_key.trim().is_empty()
                })
                .map(|p| {
                    BailianChatPolish::new(
                        p.api_key.clone(),
                        BailianChatPolish::default_chat_base(),
                        if cfg.polish_cloud_model.trim().is_empty() {
                            "qwen-turbo".into()
                        } else {
                            cfg.polish_cloud_model.clone()
                        },
                    )
                })
        }
    }

    /// 是否配置了可用云端 key（翻译 / QA 启动前的「可否开始」检查）。
    pub fn has_cloud_key(&self) -> bool {
        let cfg = self.config.blocking_read();
        let independent = !cfg.polish_cloud_endpoint.trim().is_empty()
            && !cfg.polish_cloud_api_key.trim().is_empty();
        let via_provider = cfg
            .providers
            .iter()
            .any(|p| p.kind == voice_core::ProviderKind::Bailian && !p.api_key.trim().is_empty());
        independent || via_provider
    }

    /// 录音插入前的润色上下文。P1 字段：intent / prefix_roles_enabled / style_packs /
    /// translate_target_lang / translate_with_polish。
    pub async fn polish_context(&self, intent: SessionIntent) -> PolishContext {
        let cfg = self.config.read().await.clone();
        // 兼容旧配置：polish_mode 缺失(Off) 但旧版 polish_enabled=true → 迁到 Light。
        let mode = if cfg.polish_mode != PolishMode::Off {
            cfg.polish_mode
        } else if cfg.polish_enabled {
            PolishMode::Light
        } else {
            PolishMode::Off
        };

        let style_prompt = if mode == PolishMode::Off {
            None
        } else {
            cfg.active_style_pack_id.as_deref().and_then(|id| {
                self.store
                    .list_style_packs()
                    .ok()
                    .and_then(|ps| ps.into_iter().find(|p| p.id == id).map(|p| p.system_prompt))
            })
        };

        let hotwords = self
            .store
            .list_hotwords()
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.word)
            .collect();
        let style_packs = self.store.list_style_packs().unwrap_or_default();

        PolishContext {
            enabled: mode != PolishMode::Off,
            mode,
            style_prompt,
            hotwords,
            timeout_ms: cfg.polish_timeout_ms.max(100),
            cancel: Some(self.polish_cancel.clone()),
            intent,
            prefix_roles_enabled: cfg.prefix_roles_enabled,
            style_packs,
            translate_target_lang: cfg.translate_target_lang.clone(),
            translate_with_polish: cfg.translate_with_polish,
        }
    }

    /// 请求停止录音（置停止标志）。
    pub fn request_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// 请求中止录音：置 abort + stop。中止 = 不上屏、不 QA 提问（仅防御）。
    /// R9 主路径永不调用；保留给其它命令/未来用，故允许未使用告警。
    #[allow(dead_code)]
    pub fn request_abort(&self) {
        self.abort_flag.store(true, Ordering::SeqCst);
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// 取出并清掉 abort 标志。返回 true 表示本轮应中止。
    pub fn take_abort(&self) -> bool {
        self.abort_flag.swap(false, Ordering::SeqCst)
    }

    pub fn clear_stop(&self) {
        self.stop_flag.store(false, Ordering::SeqCst);
        // R9：clear_stop 同时清 abort（CAS 成功后、任何 await 之前调用一次）。
        self.abort_flag.store(false, Ordering::SeqCst);
    }

    /// R2:请求取消进行中的润色（ESC 触发）。
    pub fn request_cancel_polish(&self) {
        self.polish_cancel.store(true, Ordering::SeqCst);
    }

    /// R2:清掉润色取消标志（每次润色开始前调用）。
    pub fn clear_cancel_polish(&self) {
        self.polish_cancel.store(false, Ordering::SeqCst);
    }
}

pub fn load_config(store: &SqliteStore) -> Result<Option<AppConfig>, Error> {
    match store.get_setting(CONFIG_KEY)? {
        Some(json) => {
            let mut cfg: AppConfig = serde_json::from_str(&json)
                .map_err(|e| Error::Store(format!("解析 app_config 失败: {e}")))?;
            // H2：provider api_key 从 keychain 填充（save 时已置空，明文不落 JSON）。
            for (i, p) in cfg.providers.iter_mut().enumerate() {
                if p.api_key.is_empty() {
                    p.api_key = crate::credentials::fetch_provider_key(i).unwrap_or_default();
                }
            }
            // PR1：polish_cloud_api_key 迁移到 keychain（若 JSON 仍残留明文）。
            if !cfg.polish_cloud_api_key.trim().is_empty() {
                let _ = crate::credentials::store_polish_key(cfg.polish_cloud_api_key.trim());
                cfg.polish_cloud_api_key.clear();
            }
            // PR1：从 keychain 回填 polish_cloud_api_key 到内存（运行时使用）。
            if cfg.polish_cloud_api_key.is_empty() {
                cfg.polish_cloud_api_key =
                    crate::credentials::fetch_polish_key().unwrap_or_default();
            }
            // R3：启动期 fail-open——清空无法通过校验的非空 endpoint（不阻断启动）。
            sanitize_endpoints(&mut cfg);
            Ok(Some(cfg))
        }
        None => Ok(None),
    }
}

pub fn save_config(store: &SqliteStore, cfg: &AppConfig) -> Result<(), Error> {
    // H2：api_key 迁移到 keychain，JSON 里置空（不存明文）。
    let mut cfg = cfg.clone();
    for (i, p) in cfg.providers.iter_mut().enumerate() {
        if !p.api_key.is_empty() {
            let _ = crate::credentials::store_provider_key(i, &p.api_key);
            p.api_key.clear();
        }
    }
    // PR1：polish_cloud_api_key 存 keychain，JSON 置空（不落明文）。
    if !cfg.polish_cloud_api_key.trim().is_empty() {
        let _ = crate::credentials::store_polish_key(cfg.polish_cloud_api_key.trim());
        cfg.polish_cloud_api_key.clear();
    }
    let json = serde_json::to_string(&cfg)
        .map_err(|e| Error::Store(format!("序列化 app_config 失败: {e}")))?;
    store.set_setting(CONFIG_KEY, &json)?;
    Ok(())
}

/// R3：启动期 fail-open 校验——把无法通过 `validate_endpoint` 的非空 URL 清空并 warn。
fn sanitize_endpoints(cfg: &mut voice_core::AppConfig) {
    use voice_core::ProviderKind;
    for p in cfg.providers.iter_mut() {
        let url = p.base_url.trim().to_string();
        if url.is_empty() {
            continue;
        }
        let target = match p.kind {
            ProviderKind::Bailian => voice_core::providers::bailian::normalize_ws_url(&url),
            ProviderKind::OpenAiAsr | ProviderKind::MultimodalAsr => url.clone(),
            ProviderKind::Sherpa => continue,
        };
        if voice_core::endpoint::validate_endpoint(&target).is_err() {
            crate::log_warn!(
                "启动时清空无效 endpoint：{}（归一化 {}）",
                p.base_url,
                target
            );
            p.base_url.clear();
        }
    }
    if !cfg.polish_cloud_endpoint.trim().is_empty()
        && voice_core::endpoint::validate_endpoint(cfg.polish_cloud_endpoint.trim()).is_err()
    {
        crate::log_warn!(
            "启动时清空无效 polish_cloud_endpoint：{}",
            cfg.polish_cloud_endpoint
        );
        cfg.polish_cloud_endpoint.clear();
    }
}
