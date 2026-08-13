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

- [x] 4.1 设置页：翻译快捷键 / 固定目标语言下拉（zh/en/ja/ko/fr/de/es）/「先润色再翻译」开关
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

- `cargo test`（workspace）：openime 21 passed · voice-core 219 passed · 集成 13 passed，0 failed
- `pnpm test`：18 passed（Settings 11 / App 4 / History 3）
- `pnpm build`（tsc + vite）：通过
- `cargo check -p openime`：无错误（仅 app_focus.m 既有 ObjC 非 ARC 告警，HEAD 既有）

## 备注（跳过 / 待人工 / 待 CI）

- **Windows 编译未验证**（macOS 上 `cfg(target_os="windows")` 不参与编译）：`insert_fallback::windows_ctrl_v`、`platform/windows/focus.rs` 按 windows 0.58 API 编写，待 GitHub Actions windows-latest 验证。
- 手工验收项（需真机/真模型）未跑：A4.1/A4.2/A4.4、A5.1/A5.2、A6.1、A6.6、A7.1–A7.5。对应自动化：A4.3/A4.4b/A4.5、A5.1b–A5.8、A6.1b/A6.2–A6.7（纯函数部分）、A7.7 均通过。
- 与设计稿的小偏差：`insert_finals_with_polish` 返回 `Vec<FinalInsertResult>`（= 文本+四态+警告 超集，覆盖设计中 `PolishOutcome.warning` 与 `InsertOutcome` 两路 HUD 映射）。
