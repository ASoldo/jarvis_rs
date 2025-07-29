# Jarvis - Offline Voice Assistant in Rust

Jarvis is a fully offline, voice-activated personal assistant built in Rust. Inspired by the Python prototype, this version offers robust performance, cross-platform compatibility, and modular integration with CLI tools, LLMs (via Ollama), and text-to-speech engines.

---

## ✨ Features

* Offline speech recognition using [Vosk](https://alphacephei.com/vosk/)
* Wake-word detection (default: `jarvis`)
* Conversational interaction powered by local LLMs (e.g., Ollama + Qwen3)
* Voice responses using RHVoice
* Tool calling support via:

  * `shell_task`: Run shell commands in a persistent working directory
  * `codex_cli_task`: Use `codex --full-auto` to scaffold code or execute tasks
* Rust-native state tracking via `~/.jarvis`:

  * `jarvis.status`, `jarvis.spoken`, `jarvis.heard`, `jarvis.working_directory`, etc.

---

## 🚀 Quickstart

### 1. Build

```bash
cargo build --release
```

Binary will be located at:

```bash
target/release/jarvis
```

### 2. Install Prerequisites

#### Required Tools:

* **[Ollama](https://ollama.com/)**

  * Used to serve local language models (e.g. Qwen3)
  * Run `ollama pull qwen3:1.7b`

* **[Vosk Model](https://alphacephei.com/vosk/models)**

  * Offline speech recognition
  * Download and extract a model, e.g.:

    ```bash
    mkdir -p ~/models && cd ~/models
    wget https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip
    unzip vosk-model-small-en-us-0.15.zip
    ```

* **[RHVoice](https://github.com/RHVoice/RHVoice)**

  * For text-to-speech output
  * Install on Arch: `yay -S rhvoice`
  * Install on Ubuntu: `sudo apt install rhvoice`

### 3. Configure `.env`

Create a file named `.env` in the project root or next to the binary:

```env
VOSK_MODEL_PATH=/home/you/models/vosk-model-small-en-us-0.15
MODEL_NAME=qwen3:1.7b
VOICE_NAME=slt
TRIGGER_WORD=jarvis
CONVERSATION_TIMEOUT=30
MIC_NAME_KEYWORD=usb
```

> You can run without `.env` by exporting the variables in your shell.

### 4. Run It

```bash
./target/release/jarvis
```

---

## 🤝 How It Works

### Wake Word

* Jarvis continuously listens for the configured `TRIGGER_WORD` (default: `jarvis`).
* When heard, it enters **conversation mode**.

### Conversation Mode

* Captures your next command
* Forwards it to a local LLM (via Ollama HTTP API)
* The LLM may:

  * Answer directly
  * Call a tool (`shell_task`, `codex_cli_task`, `persistent_shell_task`)
* Response is spoken via RHVoice

### Files in `~/.jarvis`

Jarvis writes runtime info to:

```bash
~/.jarvis/
├── jarvis               # PID
├── jarvis.status        # idle, listening, speaking, canceled
├── jarvis.spoken        # last spoken text
├── jarvis.heard         # last input
├── jarvis.working_directory  # used by tools to persist current dir
```

---

## 🛋️ Background / Auto-start

You can background Jarvis with:

```bash
nohup ./jarvis > ~/.jarvis/jarvis.log 2>&1 &
```

Or create a systemd service:

```ini
[Unit]
Description=Jarvis Voice Assistant

[Service]
ExecStart=/path/to/jarvis
EnvironmentFile=/path/to/.env
Restart=always

[Install]
WantedBy=default.target
```

---

## 🌐 Cross-Platform Notes

| Platform | Notes                                               |
| -------- | --------------------------------------------------- |
| Linux    | Fully supported                                     |
| macOS    | Vosk + RHVoice may require extra setup              |
| Windows  | Vosk works, RHVoice requires custom TTS integration |

> Future versions may include native TTS support for Windows/macOS.

---

## 📊 Project Structure

| File            | Purpose                             |
| --------------- | ----------------------------------- |
| `main.rs`       | Entry point and event loop          |
| `agent.rs`      | LLM interaction and tool invocation |
| `speech.rs`     | Microphone listening with Vosk      |
| `tts_engine.rs` | Voice output via RHVoice            |
| `tools.rs`      | Custom Rust tools for shell + codex |
| `jarvis_io.rs`  | IO handling for `.jarvis` folder    |

---

## 🔧 Roadmap

* [ ] Add GUI tray / status overlay
* [ ] Support Windows/macOS native voices
* [ ] Add hotkey to toggle Jarvis on/off
* [ ] Use Whisper or other models optionally

---

## 😎 Credits

* Built in Rust by @ASoldo
* Inspired by [Jarvis Python prototype](https://github.com/llm-guy)
* Uses:

  * [Vosk](https://github.com/alphacep/vosk-api)
  * [Ollama](https://ollama.com)
  * [RHVoice](https://github.com/RHVoice/RHVoice)

---

## 🚫 Disclaimer

This is a local/offline assistant. It does **not** send any data to the cloud. Use at your own risk.

---

## ✨ License

MIT
