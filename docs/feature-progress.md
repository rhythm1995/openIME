# 功能实现进度

> 基线：`docs/competitive-research.md` 的 26 项可借鉴功能。逐项 TDD 实现，完成一项勾一项。
> 不适用项（架构原因）标注后跳过。

## ✅ 已完成
- [x] **L0 热词同音纠错**（12150a3）— 拼音滑窗同音替换（制谱→智谱）
- [x] **B4 末尾标点去除**（a90d478）— 单句输入不补/去句末标点
- [x] **B1 数字 ITN**（22d4d3e）— 中文数字→阿拉伯（0-99 + 纯串 + 百分之）
- [x] **D2 历史搜索后端**（abb0f78）— store.search_utterances LIKE
- [x] **F1 风格包**（46a2e28 / f17f998 / 4c8007f）— store + 后端全链路 + 前端选择 UI
- [x] **B5 按 app 标点** — 全角→半角（IM 场景），按前台 app bundle 偏好转换上屏
- [x] **B6 繁简转换** — ferrous-opencc（纯 Rust OpenCC），按偏好简↔繁转换上屏
- [x] **A1 按住说话 PTT** — HotkeyMode（Toggle/Hold），Hold 模式 press 只开始不切换停
- [x] **H2 凭据钥匙串** — keyring crate，API key 存 keychain 不落明文 JSON
- [x] **F4 选区注入** — macOS AX 直读 AXSelectedText（不碰剪贴板），get_selection 命令
- [x] **D3 文件转录** — symphonia 解码 + 重采样 16k + sherpa 整段 + srt + 前端选择/导出
- [x] **D1 日记归档** — store.export_diary_markdown 按日期分组 + export_diary 命令
- [x] **B2 音素热词模糊音** — normalize_fuzzy（zh→z/sh→s/ch→c/en→en/ing→in）归一匹配

## ⚠️ 不适用（架构原因）
- C2 剪贴板恢复 — enigo 逐字输入不碰剪贴板
- H3 ESC 中断 LLM — polish 是一次性 + timeout（非流式 token）

## 📋 待做

### 轻量后端
（全部完成 ✅）

### 前端补全
- [x] **D2 搜索 UI**（前端搜索框 + 跨会话 LIKE 结果）
- [x] **F1 风格包 CRUD + 快捷键**（自定义增删 + 全局快捷键运行时切换）

### 大工程（周级）
- [ ] **C1 流式逐字重做**（OpenLess 三态机 + Unicode 边界）
