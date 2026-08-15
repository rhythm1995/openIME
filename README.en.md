**English** | [简体中文](./README.md)

<p align="center">
  <img src="branding/app-icon.svg" width="100" height="100" alt="openIME logo" />
</p>
<h1 align="center">openIME</h1>
<p align="center">An open-source, local-first, cross-platform <strong>voice input method</strong> — press a shortcut, speak, and recognized text is typed at your cursor.</p>

<p align="center">
  <a href="./releases">Download</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="docs/user-guide.md">User Guide</a> ·
  <a href="docs/development.md">Development</a> ·
  <a href="docs/troubleshooting.md">Troubleshooting</a>
</p>

<br />

<p align="center">
  <img src="docs/screenshots/en.png" alt="openIME main window" width="760" />
</p>

> [!NOTE]
> Lives in the menu bar. Press the global shortcut (default **Fn / 🌐 key**) to record → real-time streaming transcription → text is typed character-by-character at the current cursor → sessions saved to history. Inspired by AutoGLM, rebuilt with Tauri + Rust as an **open-source, localizable, testable** implementation.

## Features

- 🔒 **Local-first, private** — On-device `sherpa-onnx` recognition; audio never leaves your machine. One-click model download (SHA256 + resumable + HF→mirror failover).
- ☁️ **Switchable engines** — Local sherpa / Bailian WebSocket streaming / OpenAI-compatible REST / Multimodal REST; engine URLs are auto-normalized.
- ✨ **3-tier AI polish** — L0 rule correction (hotword homophone/fuzzy, numeral ITN, simplified/traditional, trailing-punctuation removal) + L2 LLM (local Qwen3.5 first — 0.8/2/4B by machine tier; cloud OpenAI Chat / Anthropic / Responses; original text passthrough on double failure).
- 📝 **Hotword dictionary & style packs** — Custom terms for pronunciation correction; custom system prompts to switch output style; prefix roles (lines starting with "邮件:" / "翻译:" are routed automatically).
- 🎬 **File transcription** — Audio file → SRT subtitles; long audio auto-segmented (adjustable duration / overlap, with progress and cancel).
- 🌐 **Speech translation & selection QA** — Dedicated shortcut: speak the source language, target language is typed at the cursor (7 base languages; cloud or a local dedicated translate model — MiLMMT-46 / HY-MT — unlocks an extended set); select text and ask questions by voice in a floating panel with streaming answers.
- 🌐 **Bilingual UI (CN/EN)** — One-click toggle at the bottom-left. Interface language is fully independent from the ASR recognition language.
- ⌨️ **Global shortcut** — Default Fn (🌐) with push-to-talk (PTT, release to stop); toggle mode optional.
- 🧲 **Insertion fallback** — If simulated typing is swallowed by the target app, falls back to paste (Cmd+V) and restores your clipboard; short Fn presses in PTT mode re-post the original 🌐 function.
- 🍎 **macOS** fully supported; 🪟 **Windows** in beta (CapsLock single-key recording and insertion fallback verified on real hardware; native TSF commit pending).

## Quick Start

> End-user perspective. For development, see [docs/development.md](docs/development.md).

1. **Install** — Download the macOS `.dmg` from [Releases](./releases) and drag it to `/Applications`. The beta build is unsigned; on first launch **right-click → Open**.
2. **Grant permissions** — Open openIME → Settings → System Permissions → authorize **Microphone** and **Accessibility**.
3. **Pick an engine** — Local sherpa-onnx is recommended by default (Settings → Recognition engine → Download model); or fill in cloud credentials.
4. **Speak to type** — Press **Fn (🌐)** to start recording, press again to stop; recognized text is typed at the current cursor.

> [!TIP]
> Want an English interface? Click the 🌐 button at the bottom-left to toggle between Chinese and English. The interface language **does not affect** the speech-recognition language — that is set separately under "Recognition engine → Default language".

## Recognition engines

| Engine | Description | Status |
|---|---|---|
| **Local sherpa-onnx** | In-process offline recognition, one-click model download; OfflineRecognizer cached (zero load on second recording) | ✅ |
| **Bailian WebSocket streaming** | Protocol A (run-task / result-generated), character-by-character display | ✅ |
| **OpenAI-compatible REST** | `POST /audio/transcriptions`, compatible with Whisper / OpenRouter | ✅ |
| **Multimodal REST** | `POST /chat/completions`, compatible with Bailian Qwen3 ASR non-streaming | ✅ |

Cloud LLM polish supports 3 protocols. Policy is fixed: "local first, auto-fallback to cloud on failure / not installed, original text passthrough on double failure — never errors".

## Architecture

All core logic lives in `voice-core` (a pure library with zero Tauri dependency). Four mockable traits form the end-to-end pipeline:

```
AudioSource ──► AsrProvider/AsrSession ──► TextInserter
   (cpal)          (Bailian WS / sherpa)       (enigo)
                        │
                        ▼
                  HistoryStore (SQLite)
```

Full directory layout, module notes, and design rationale in [docs/development.md](docs/development.md).

## Development

```bash
pnpm install && pnpm test       # Frontend (Vitest + React Testing Library)
cargo test -p voice-core        # Core library
./scripts/build.sh install      # Build and install to /Applications (fixed signing)
```

392 tests run locally (voice-core 345 = lib 332 + integration 13; frontend 47); src-tauri app-shell tests run in Windows CI (84). Full dev workflow, test matrix, CI, signing, and releases in [docs/development.md](docs/development.md); logs & troubleshooting in [docs/troubleshooting.md](docs/troubleshooting.md).

## Roadmap

- ✅ Engines, overlay, tray, global shortcut, onboarding, history details, UI i18n (Chinese / English)
- ✅ 3-tier AI polish, hotword dictionary, style packs + prefix roles, translation mode, selection QA panel, file transcription (long-audio segmentation), simplified/traditional, selection injection, journal export, machine-profile → model-fit tagging, local model suite (ASR + polish + translate, one-click tiered download)
- ✅ Single-instance lock, ESC interrupt, endpoint SSRF validation, paste fallback + clipboard restore, Fn short-press repost
- 🔜 Windows (NSIS packaging + real-machine e2e done; TSF native integration pending)

> Full requirements backlog with implementation status (in Chinese): [docs/roadmap.md](docs/roadmap.md).

## Acknowledgements

- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) — on-device ASR engine
- [Tauri](https://tauri.app/) · [enigo](https://github.com/enigo-rs/enigo) · [cpal](https://github.com/RustAudio/cpal)
- Product form inspired by AutoGLM

---

<p align="center"><sub>Local-first · Open source · Testable</sub></p>
