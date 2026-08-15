# openIME 实现进度（TDD）

## P2（[`p2-design.md`](./p2-design.md)）

跟踪 P2 的 PR0–PR6。每条用 TDD，完成一项打勾；完成一个 PR 跑全量 `cargo test -p voice-core` + `cargo test -p openime --lib` + `pnpm test` + `pnpm build`。

依赖图：PR0 → PR1 ∥ PR2 ∥ PR4；PR2 → PR3；PR4 → PR5 → PR6。

| PR | 主题 | 状态 |
|---|---|---|
| PR0 | P2 AppConfig 字段（serde default + validate_p2_fields） | ✅ 完成 |
| PR1 | R12 长音频分段 + 重叠 + 进度/取消 | ✅ 完成 |
| PR2 | R9 Hold+Fn delay-start + 修 Toggle 松开停 | ✅ 完成 |
| PR3 | R9 macOS flagsChanged 补发 + 吞键 tap | ✅ 完成（真机 A9.1 待验） |
| PR4 | R11 Windows TSF 阶段 A（HKCU + DLL + NSIS） | ⏭️ 跳过（需 Windows 编译 + 真机） |
| PR5 | R11 TSF 命名管道 CommitText 协议 | ✅ 纯协议部分完成（Windows FFI 待 Windows） |
| PR6 | R11 insert_ex 优先 TSF + R7 回退 | ✅ 纯函数部分完成（Windows FFI 待 Windows） |

---

## PR0 — P2 AppConfig 字段 ✅

- [x] 0.1 `config.rs`：7 个 P2 字段全部 `#[serde(default)]`（short_press_ms / fn_repost_enabled / fn_repost_tis_fallback / windows_tsf_enabled / windows_tsf_fallback / file_seg_duration_secs / file_seg_overlap_secs）
- [x] 0.2 `AppConfig::validate_p2_fields`：范围校验（short_press_ms 100..=800、duration 10..=180、overlap 1..=30 且 overlap<duration）
- [x] 0.3 `commands.rs save_app_config`：接入 `validate_p2_fields`，失败整单不落盘
- [x] 0.4 `types.ts` / `Settings.test.tsx` `defaultConfig` 同步 7 字段
- [x] 0.5 i18n zh/en：短按阈值 + fnRepost + 分段时长/重叠 + 进度/取消占位键；`Settings.tsx` 增加短按阈值输入
- [x] 0.6 验收：voice-core 232 passed · openime 22 passed · pnpm 18 passed · tsc 通过

## PR1 — R12 长音频分段 + 重叠 ✅

- [x] 1.1 `transcribe.rs segment_ranges`：0/短/60s/64s/1800s/非法 Err，**无**并入分支（短尾段自然出现）
- [x] 1.2 `stitch_overlap` / `stitch_overlap_punct`：k_min=2、标点二次去重、单字不误吃、空串
- [x] 1.3 `srt_from_segments`（时间戳 + t0、i>0 丢 start<t0+overlap/2、跨段序号连续）+ `transcribe_segmented`
- [x] 1.4 `transcribe_file_full` 新签名（seg/overlap/cancel/on_progress）；一次 `build_offline_recognizer` 顺序喂切片后 drop；**不碰** `OFFLINE_RECOGNIZER_CACHE`
- [x] 1.5 `lib.rs`（voice-core）导出 segment_ranges / stitch_overlap / stitch_overlap_punct / srt_from_segments / transcribe_segmented
- [x] 1.6 `state.rs` transcribe_guard/transcribe_cancel + `commands.rs` transcribe_file（CAS guard + 段间 emit 进度）/ cancel_transcribe
- [x] 1.7 前端：`ipc.ts` cancelTranscribe；`Settings.tsx` 分段时长/重叠输入 + `transcribe://progress` 监听 + 取消按钮 + 转录中 disable
- [x] 1.8 验收：mock 33 段 stitch、mock 取消（第 2 段后 → Err「已取消」）、srt 序号连续 + overlap/2 丢 cue 全过

## PR2 — R9 Hold+Fn delay-start + 修 Toggle 松开停 ✅

- [x] 2.1 `fn_policy.rs`（新）：`classify_fn_edge` 状态机 + `should_ignore_fn_edge`（表驱动 11 case + ignore window 单测）
- [x] 2.2 `lib.rs on_fn_edge`：delay-start（`ArmHoldTimer` → 阈值到期仍按住才 `StartRecord`）+ `this_press_started_recording`；Toggle 松开不再停（KD-2）
- [x] 2.3 `state.rs`：`abort_flag` / `request_abort` / `take_abort`；`clear_stop` 同时清 abort
- [x] 2.4 `commands.rs`：CAS 成功后、任何 await 之前立刻 `clear_stop`；两处防御 `take_abort`（音频创建后、`record_and_collect` 返回后）
- [x] 2.5 阈值前不 `mark_recording`：delay-start 使 `on_record_hotkey` 仅在阈值后调用（QA 短按不进 `QaRecording`，S9.4）
- [x] 2.6 验收：openime 24 passed（含 fn_policy 2 项）；补发仍为 log（PR3 落地）
- [x] 2.7 `fn_policy::fn_tap_can_consume(hotkey, hold)` 纯判定 + 单测（只改 `hotkey_mode` 不 `request_stop` / 吞键判定翻转）

## PR3 — R9 macOS flagsChanged 补发 + 吞键 tap ✅

- [x] 3.1 `fn_monitor.m`：Default tap（可吞键）+ 补发一对 `kCGEventFlagsChanged`（双 magic `0x4F494D45`）+ `REPOST_IGNORE_MS=60` 主过滤 + NSEvent 同 ignore window + 失败退回 ListenOnly
- [x] 3.2 `fn_key.rs` FFI：`set_fn_tap_consume` / `schedule_repost_fn` / `repost_fn`
- [x] 3.3 `lib.rs`：`RepostOnly → schedule_repost_fn`；`store_fn_tap_consume(hotkey==Fn && Hold)` 在 `save_app_config` + `apply_hotkey` 下发
- [x] 3.4 Windows / 非 macOS 平台桩（保证跨平台编译）
- [x] 3.5 验收：`cargo check -p openime` 通过（ObjC 编译）；openime 24 passed
- [x] 3.6 `Settings.tsx`「Hold 下短按 Fn 补发 🌐」开关（仅 macOS 渲染，`IS_MAC` 判定）
- [ ] 3.7 待真机：HID flagsChanged 补发是否触发 🌐 切输入法（A9.1，Open Question 1）；TIS 回退 `fn_repost_tis_fallback` 未接（默认 false）

## PR5 — R11 TSF 命名管道协议（纯协议部分）✅

- [x] 5.1 `windows_ime/protocol.rs`：常量（LANG 0x0804 / CLSID / PROFILE_GUID / 管道前缀 / 协议版本 1）+ `ImeProtocolMessage`（tag=type + camelCase + rename_all_fields）
- [x] 5.2 `windows_ime/profile.rs`：`ImeProfileSnapshot` / `ProfileRestoreDecision` / `restore_decision`（A11.3）
- [x] 5.3 `should_fallback_after_ime`（A11.4）+ `ime_pipe_name_for_target`（含 pid-tid，A11.5）
- [x] 5.4 黄金 fixture 4 条（`windows-ime/fixtures/*.json`）+ roundtrip / stale session / errorCode 蛇形单测（A11.9）
- [x] 5.5 验收：openime 34 passed（含 windows_ime 10 项）
- [ ] 5.6 待 Windows：`windows_ime/{session,ipc}.rs` FFI（WaitNamedPipe + CreateFile 重试 800ms + `GetNamedPipeServerProcessId` 校验 + WM_INPUTLANGCHANGEREQUEST）；C++ `ipc_server.cpp` / `edit_session.cpp`

## PR6 — R11 insert_ex 优先 TSF（纯函数部分）✅

- [x] 6.1 `insert.rs`：`InsertOutcome::Committed`（HUD 与 Typed 一样静默）
- [x] 6.2 `InsertOpts` 增 `tsf_enabled` / `tsf_fallback`；`InsertOpts::from_config(cfg, frontmost, streaming)` 唯一业务构造（`tsf = windows && cfg && !streaming`）
- [x] 6.3 `commands.rs toggle_recording` + `qa.rs insert_last_answer` 都改走 `from_config`；streaming 提前计算供组装
- [x] 6.4 验收：voice-core 234 passed · openime 34 passed；`from_config` 单测（非 Windows / streaming 时 tsf_enabled=false）
- [x] 6.5 `tsf_supported_for_machine`（仅 AMD64 走 TSF；I386 / ARM64 / None → R7，A11.10）+ 单测
- [ ] 6.6 待 Windows：`insert_fallback.rs insert_ex` 的 TSF 通路（prepare → 目标管道 ClientReady → SubmitText → Committed → 失败回退 R7）；`focus.rs frontmost_process_info`（pid/tid/machine 门控）；设置页 IME 状态 +「恢复系统输入法」按钮

## P2 跳过 / 待 Windows / 待真机

> 2026-08-14 更新（`4c0845e` Windows 移植后）：src-tauri 已在 Windows 真机编译 / 打包（NSIS）/ 运行期 e2e 通过，CI 新增 `tauri-shell-windows` job（clippy -D warnings + cargo test）常态兜底；Windows 侧 Fn/CapsLock 单键录音、单实例、UIA 选区已实现。TSF（PR4–PR6 的 Windows FFI 与 C++ DLL）仍待落地，细节见 [openIME-windows-porting-notes.md](../openIME-windows-porting-notes.md)。

- **PR4 全部跳过**（需 Windows 编译）：C++ `OpenImeTsf.dll`（ITfTextInputProcessorEx + ITfEditSession，`/MT`）、`CMakeLists.txt`、NSIS `hooks.nsh`（HKCU `regsvr32 /s`）、`tauri.conf.json` `resources + installerHooks`、`windows_ime_status` 探测。本机 macOS 无法编译/验证，按「不确定先跳过」。
- **PR5/PR6 的 Windows FFI 部分跳过**（同上）：命名管道 client/session、`frontmost_process_info`、`insert_ex` TSF 分支、设置页 TSF 开关与恢复按钮。纯协议/纯函数已落地并单测。
- **PR3 真机验收待跑**：HID flagsChanged 补发是否触发 🌐（A9.1）；`fn_repost_tis_fallback` 未接（默认 false）。

---

# P1 实现进度（TDD）

跟踪 [`p1-design.md`](./p1-design.md) 的 PR1–PR6。每条用 TDD，完成一项打勾；完成一个 PR 跑全量 `cargo test -p voice-core` + `pnpm test`。

依赖图：PR1 ∥ PR2 → PR3 → PR4 → PR5 / PR6。

| PR | 主题 | 状态 |
|---|---|---|
| PR1 | R3 endpoint SSRF 校验 + polish_cloud Keychain | ✅ 完成 |
| PR2 | R7 粘贴兜底 + 剪贴板恢复（macOS Cmd+V / Windows Ctrl+V） | ✅ 完成 |
| PR3 | LlmClient（translate / chat SSE / PolishOutcome） | ✅ 完成 |
| PR4 | R4 翻译快捷键 + hotkey 注册中心 + SessionIntent | ✅ 完成 |
| PR5 | R5 前缀角色（带 match_prefix 的风格包） | ✅ 完成 |
| PR6 | R6 划词语音问答浮窗 | ✅ 完成 |

---

## PR1 — endpoint SSRF + Keychain ✅

- [x] 1.1 `crates/voice-core/src/endpoint.rs`：`validate_endpoint` + 辅助函数 + 32 case（含 mapped IPv6 / IMDS / 0/8 / decimal）
- [x] 1.2 `crates/voice-core/src/http.rs`：`http_client_no_redirect`
- [x] 1.3 `voice-core/Cargo.toml`：`url = "2"`
- [x] 1.4 `config.rs` `ProviderConfig::validate`：非空 URL（百炼验归一化、REST 验原文）
- [x] 1.5 `providers/{openai_asr,multimodal_asr}.rs` + `polish/cloud.rs`：走 no-redirect client（5 处）
- [x] 1.6 `commands.rs` `save_app_config` + `validate_all_endpoints`：校验所有非空用户 URL（不强制 api_key）
- [x] 1.7 `state.rs` `sanitize_endpoints`：启动期坏 URL 清空 + warn
- [x] 1.8 `credentials.rs` `store_polish_key` / `fetch_polish_key` + load/save 迁移（JSON 明文→Keychain）
- [x] 1.9 验收：voice-core 153 passed · openime credentials 2 passed · check 无错

## PR2 — 粘贴兜底 + 剪贴板恢复 ✅

- [x] 2.1 voice-core `insert.rs`：`InsertOutcome` 四态 + `should_restore_clipboard` 纯函数（8 测试）
- [x] 2.2 src-tauri `insert_fallback.rs`：`arboard` + 平台粘贴和弦（macOS CGEvent Cmd+V / Windows enigo Ctrl+V）+ `PendingRestore` 状态机 + `CLIPBOARD_MU`（RESTORE_DELAY=750ms，恢复线程 catch_unwind）
- [x] 2.3 Windows 焦点模块 `platform/windows/{mod,focus,fn_key,permissions}.rs`：`frontmost_exe_basename` / `activate_by_exe_basename`（GetForegroundWindow→OpenProcess+QueryFullProcessImageNameW→EnumWindows+SetForegroundWindow）+ 假 HWND 单测。**macOS 无法编译验证，Windows 侧待 CI 验证**
- [x] 2.4 pipeline `insert_opts`（`InsertOpts` 独立参数）+ config `InsertStrategy`/`paste_fallback_apps`/`restore_clipboard` + 流式 chunk 失败贴 diff 一次（FR-7.10）
- [x] 2.5 前端 types.ts + Settings（插入策略/兜底 app 列表/恢复剪贴板）+ i18n + Settings.test
- [x] 2.6 验收 A7（状态机：连续粘贴 original 取第一次 / 用户中途复制不覆盖 / Type 不碰 PENDING / mstsc.exe 命中 / macOS=Cmd+V 单测）

## PR3 — LlmClient ✅

- [x] 3.1 `polish/llm.rs`：`LlmClient` trait（polish / translate_text / polish_and_translate / chat_stream）+ `TranslateRequest` / `ChatRequest{cancel,gen,on_delta}` / `PolishTranslate` + `parse_sse_line`（delta/[DONE]/finish_reason，fixture 双 delta 拼全文）
- [x] 3.2 `polish/cloud.rs`：三协议 translate / polish_and_translate（复用 `post_json`）；`max_tokens` 按请求传入（PolishRequest.max_tokens）；QA SSE 仅 OpenAI Chat（stream=true 逐行解析 + cancel 中断）
- [x] 3.3 `polish/prompts.rs`：`lang_display_name` 语言表 + 翻译 / 润色+翻译 prompt + 哨兵 `[[OPENIME_POLISHED_SOURCE]]`/`[[OPENIME_TRANSLATION]]` + `parse_polish_translate`
- [x] 3.4 `PolishOutcome{text,warning}` / `PolishWarn{TranslateFailed,RoleLlmFailed,RoleNoBackend}`（pipeline.rs）

## PR4 — 翻译 + hotkey 中心 ✅

- [x] 4.1 设置页：翻译快捷键 / 目标语言下拉 /「先润色再翻译」开关（P1 范围为固定 7 语 zh/en/ja/ko/fr/de/es；本地三件套后续扩展为基础 7 语 + 扩展集，见下文「本地三件套实现进度」节）
- [x] 4.2 `SessionIntent{Dictate,Translate,Qa}` + `pending_intent`（guard 抢到后 take，失败清回 Dictate）
- [x] 4.3 `lib.rs` 注册中心收口：`apply_hotkey(cfg)` 注册 录音/风格/翻译/QA；`parse_code` 扩展 `;`/`'`/`[`/`]`/`,`/`.`/`/`/`=`/`-` + `Cmd+Shift+;` 单测
- [x] 4.4 `on_hotkey` 分流 + 互斥表（听写中翻译/QA 键拒绝+toast；QA 打开时翻译键忽略；风格循环允许）+ `toast://info`（App.tsx 监听）
- [x] 4.5 `save_app_config`：热键两两不等 + 可解析（A4.5：翻译==录音 保存失败，5 单测）；任意热键变则 `apply_hotkey`
- [x] 4.6 `toggle_recording` 分支表：Translate `streaming_insert=false`、`engine=translate`、无 key 不写 intent 不录音、失败回退 L0 原文 + `TranslateFailed` HUD、`max(8000, timeout)` / 1024
- [x] 4.7 pipeline Translate 分支（不走前缀/风格包/Router）+ 哨兵合成解析失败回退纯翻译再回退 L0（A4.4b 3 测试）

## PR5 — 前缀角色 ✅

- [x] 5.1 `store.rs` v4 迁移（match_prefix/provider/model/role_kind/output_mode）+ v3→v4 旧行默认测试 + `RoleKind`/`OutputMode`
- [x] 5.2 `polish/roles.rs`：**唯一**检测实现 `detect_prefix_role`（最长别名 / `prefix_boundary_ok` / 空正文拒绝 / 等长取小 ord / 中英冒号 / MAIL 大小写，14 测试）
- [x] 5.3 pipeline：L0 后、L2 前命中 → 直连 cloud/local（**禁止** PolishRouter）；失败去前缀原文 + `RoleLlmFailed`/`RoleNoBackend`（A5.5/A5.6/A5.7 均测）
- [x] 5.4 `role_kind=Translate` → 与 R4 共用 `translate_text` + `translate_target_lang`（A5.2b/A5.3 测试）
- [x] 5.5 `prefix_roles_enabled` → 听写 `streaming_insert=false`（A5.8）；命中时跳过 ≤8 字 L2 门
- [x] 5.6 `cycle_style_pack` 排除前缀包；seed `builtin-role-{mail,translate,cmd}`（按 id 补缺失、不覆盖用户清空前缀）
- [x] 5.7 upsert 拒绝相同别名（忽略大小写，单测）；Settings「角色 / 风格包」卡片移出 Heavy（徽章「前缀: xx」、就地编辑前缀/prompt/provider）

## PR6 — QA 面板 ✅

- [x] 6.1 `tauri.conf.json` 第三窗 `qa`（400×520、alwaysOnTop、系统标题栏）+ capabilities `"qa"` + `main.tsx#qa` 路由 + `QaPanel.tsx`
- [x] 6.2 `qa.rs` 状态机（Hidden/Idle/Recording/Transcribing/Streaming + panel_visible + 开窗时冻结 frontmost/selection）
- [x] 6.3 开窗：`get_selection` → 4000 截断（首2000+尾2000，信封用同一结果）→ Regular+show+set_focus；指针屏右下角 24px、记住位置；CloseRequested → `close_qa_panel`（关窗清空、main 隐藏则 Accessory）
- [x] 6.4 录音键分流：窗可见时 = QA 录音（`streaming_insert=false`、不还焦、HUD「问答录音中…」）；流式中再按 = 取消（bump gen，保留已输出）；ESC 动态注册取消
- [x] 6.5 `chat_stream`（60s/2048）→ `qa://delta`；8 轮/8000 字截断；首轮 `<selected_text>` 信封（闭标签转全角防投毒）+「刷新选区」后重带
- [x] 6.6 按钮：复制 / 插入光标（还焦开窗时 frontmost → R7 四态）/ 刷新选区 / 清空；无云端 key 横幅 + toast 不录
- [x] 6.7 `qa_save_history`：面板每次打开建一条 sessions（engine=qa），每轮 Q:/A: 两条 utterances seq 递增
- [x] 6.8 前端 A6.1b 状态机测试（trim/信封/第二轮含第一轮）+ qa://state 事件

## 全量验证

- `cargo test -p voice-core`：voice-core 345 passed（lib 332 + 集成 13）；openime 应用壳 macOS 不跑（windows_ime FFI 门控待 Windows），由 Windows CI 跑（84 测试函数）
- `pnpm test`：47 passed（Settings 37 / App 5 / History 3 + 其他）

> 以上为 2026-08-16 实测（本地模型三件套 `f43012f` 落地后 + 翻译目标语言分档改动）。
- `pnpm build`（tsc + vite）：通过
- `cargo check -p openime`：无错误（仅 app_focus.m 既有 ObjC 非 ARC 告警，HEAD 既有）

## 备注（跳过 / 待人工 / 待 CI）

- **Windows 编译已验证**（2026-08-14，`4c0845e`）：Windows 真机 `cargo check`/`tauri build`（NSIS）+ 运行期 e2e 通过；CI `tauri-shell-windows` job 常态兜底。`insert_fallback::windows_ctrl_v`、`platform/windows/focus.rs` 等 windows 0.58 API 均已实编验证。
- 手工验收项（需真机/真模型）未跑：A4.1/A4.2/A4.4、A5.1/A5.2、A6.1、A6.6、A7.1–A7.5。对应自动化：A4.3/A4.4b/A4.5、A5.1b–A5.8、A6.1b/A6.2–A6.7（纯函数部分）、A7.7 均通过。
- 与设计稿的小偏差：`insert_finals_with_polish` 返回 `Vec<FinalInsertResult>`（= 文本+四态+警告 超集，覆盖设计中 `PolishOutcome.warning` 与 `InsertOutcome` 两路 HUD 映射）。

---

# 本地三件套实现进度

跟踪 [`local-model-suite-plan.md`](./local-model-suite-plan.md)（需求 + 技术方案合一）。已落地（`f43012f`）。

| PR | 主题 | 状态 |
|---|---|---|
| P1 | ASR 下架 FireRed + 清 Settings fallback；`open_model_directory` + 打开目录按钮 | ✅ 完成 |
| P2 | `llm_catalog`（润色 3 档 + 翻译 2 档）+ 多 GGUF 下载 + SHA256 表 | ✅ 完成 |
| P3 | `GgufRuntime` 常驻（润色 + 翻译槽）；换档不每次 load | ✅ 完成 |
| P4 | `TranslateRouter` + `apply_translate` 两步 Light；兼译；config 新字段 | ✅ 完成 |
| P5 | combo 打标 + 推荐器写默认 + 预算条 + 弱机兼提示 | ✅ 完成 |

- **润色**：`qwen3.5-0.8b` / `qwen3.5-2b`（默认）/ `qwen3.5-4b`；加载失败回退 Qwen3-0.6B / 1.7B / 4B-Instruct-2507。旧 `qwen2.5-1.5b-*` 配置读入时映射到 `qwen3.5-2b`。
- **翻译**：`milmmt-1b`（默认专翻）/ `hy-mt-1.8b`（自选）；MiLMMT-46 不含乌克兰语 → 通用模板回退，HY-MT 中文变体（简/繁/粤）走中文模板。
- **翻译目标语言分档**（本次语言分档改动 `Settings.tsx` / `prompts.rs`）：纯本地润色模型兼译 = 基础 7 语；启用云端或本地专翻 = 扩展集（含繁中 / 粤语 / 阿拉伯 / 俄 / 葡 / 印地 / 越南 / 波兰 / 波斯 / 乌兹等约 20 种）。
- **测试**：`cargo test -p voice-core` 345（lib 332 + 集成 13，含 `llm_catalog` 12 / `runtime` 7 / `translate_router` 2 / `system` 22 等）；`pnpm test` 47。
