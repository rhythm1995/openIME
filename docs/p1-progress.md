# P1 实现进度（TDD）

跟踪 [`p1-design.md`](./p1-design.md) 的 PR1–PR6。每条用 TDD，完成一项打勾；完成一个 PR 跑全量 `cargo test -p voice-core` + `pnpm test`。

依赖图：PR1 ∥ PR2 → PR3 → PR4 → PR5 / PR6。

| PR | 主题 | 状态 |
|---|---|---|
| PR1 | R3 endpoint SSRF 校验 + polish_cloud Keychain | ✅ 完成 |
| PR2 | R7 粘贴兜底 + 剪贴板恢复（macOS Cmd+V / Windows Ctrl+V） | ⬜ |
| PR3 | LlmClient（translate / chat SSE / PolishOutcome） | ⬜ |
| PR4 | R4 翻译快捷键 + hotkey 注册中心 + SessionIntent | ⬜ |
| PR5 | R5 前缀角色（带 match_prefix 的风格包） | ⬜ |
| PR6 | R6 划词语音问答浮窗 | ⬜ |

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

## PR2 — 粘贴兜底 + 剪贴板恢复

- [x] 2.1 voice-core `insert.rs`：`InsertOutcome` 四态 + `should_restore_clipboard` 纯函数（8 测试）
- [ ] 2.2 src-tauri `insert_fallback.rs`：`arboard` + 平台粘贴和弦（macOS Cmd+V / Windows Ctrl+V）+ `PendingRestore` 状态机
- [ ] 2.3 Windows 焦点模块（Win32 exe basename，**macOS 无法编译验证**）
- [ ] 2.4 pipeline `insert_opts` + config `InsertStrategy`/`paste_fallback_apps`/`restore_clipboard`
- [ ] 2.5 前端 types.ts + Settings + i18n
- [ ] 2.6 验收 A7

## PR3 — LlmClient
（依赖 PR1）

## PR4 — 翻译 + hotkey 中心
（依赖 PR3）

## PR5 — 前缀角色
（变基 PR4）

## PR6 — QA 面板
（依赖 PR4）
