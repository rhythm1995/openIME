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

## ⚠️ 不适用（架构原因）
- C2 剪贴板恢复 — enigo 逐字输入不碰剪贴板
- H3 ESC 中断 LLM — polish 是一次性 + timeout（非流式 token）

## 📋 待做

### 轻量后端
- [ ] **H2 凭据钥匙串**（keyring）
- [ ] **F4 选区注入**（macOS AX 直读选中文字）
- [ ] **A1 按住说话 PTT**（fn_key press/release 边沿 + 模式）

### 前端补全
- [ ] **D2 搜索 UI**（前端搜索框 + 列表）
- [ ] **F1 风格包 CRUD + 快捷键**（自定义增删 + 运行时切换）

### 大工程（周级）
- [ ] **D3 文件转录**（ffmpeg + srt/txt/json）
- [ ] **C1 流式逐字重做**（OpenLess 三态机 + Unicode 边界）
- [ ] **B2 音素热词完整 RAG**（CapsWriter 两阶段 + 模糊音权重）
- [ ] **D1 日记归档**（Markdown + 音频回放链接）
