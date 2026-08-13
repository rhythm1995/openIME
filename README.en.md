**English** | [简体中文](./README.md)

<p align="center">
  <img src="branding/app-icon.svg" width="100" height="100" alt="openIME logo" />
</p>
<h1 align="center">openIME</h1>
<p align="center">An open-source, local-first, cross-platform <strong>voice input method</strong> — press a shortcut, speak, and recognized text is typed at your cursor.</p>

<p align="center">
  <a href="./releases">Download</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="docs/development.md">Development</a> ·
  <a href="docs/troubleshooting.md">Troubleshooting</a>
</p>

<br />

<p align="center">
  <img src="docs/screenshots/main.png" alt="openIME main window" width="760" />
</p>

> [!NOTE]
> Lives in the menu bar. Press the global shortcut (default **Fn / 🌐 key**) to record → real-time streaming transcription → text is typed character-by-character at the current cursor → sessions saved to history. Inspired by AutoGLM, rebuilt with Tauri + Rust as an **open-source, localizable, testable** implementation.

## Features

- 🔒 **Local-first, private** — On-device `sherpa-onnx` recognition; audio never leaves your machine. One-click model download (SHA256 + resumable + HF→mirror failover).
- ☁️ **Switchable engines** — Local sherpa / Bailian WebSocket streaming / OpenAI-compatible REST / Multimodal REST; engine URLs are auto-normalized.
- ✨ **3-tier AI polish** — L0 rule correction (hotword homophone/fuzzy, numeral ITN, simplified/traditional, trailing-punctuation removal) + L2 LLM (local Qwen2.5 first; cloud OpenAI Chat / Anthropic / Responses; original text passthrough on double failure).
- 📝 **Hotword dictionary & style packs** — Custom terms for pronunciation correction; custom system prompts to switch output style.
- 🎬 **File transcription** — Audio file → SRT subtitles.
- 🌐 **Bilingual UI (CN/EN)** — One-click toggle at the bottom-left. Interface language is fully independent from the ASR recognition language.
- ⌨️ **Global shortcut** — Default Fn (🌐); toggle or push-to-talk (PTT) modes.
- 🍎 **macOS** (Windows in progress).

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

156 tests cover the codebase (voice-core 141 + integration 8 + frontend 7). Full dev workflow, test matrix, CI, signing, and releases in [docs/development.md](docs/development.md); logs & troubleshooting in [docs/troubleshooting.md](docs/troubleshooting.md).

## Roadmap

- ✅ Engines, overlay, tray, global shortcut, onboarding, history details
- ✅ 3-tier AI polish, hotword dictionary, style packs, file transcription, simplified/traditional, selection injection, journal export, machine-profile → model-fit tagging
- 🌐 UI i18n (Chinese / English)
- 🔜 Windows

## Acknowledgements

- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) — on-device ASR engine
- [Tauri](https://tauri.app/) · [enigo](https://github.com/enigo-rs/enigo) · [cpal](https://github.com/RustAudio/cpal)
- Product form inspired by AutoGLM

---

<p align="center"><sub>Local-first · Open source · Testable</sub></p>
