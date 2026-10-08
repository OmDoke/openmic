# 🎙️ OpenMic — Stealth AI Copilot for Live Technical Interviews & Meetings

<p align="center">
  <img src="src/assests/icons/app-icon.png" alt="OpenMic Logo" width="96" height="96" onerror="this.style.display='none'"/>
</p>

<p align="center">
  <strong>An ultra-low-latency, invisible desktop copilot designed for live coding, technical interviews, and meetings.</strong><br>
  Built with <strong>Tauri v2</strong>, <strong>Rust</strong>, and <strong>Google Gemini</strong>.
</p>

<p align="center">
  <a href="#key-features">Key Features</a> •
  <a href="#architecture">Architecture</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#global-shortcuts">Global Shortcuts</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#troubleshooting">Troubleshooting</a>
</p>

---

## ✨ Key Features

- 🕵️ **Stealth Screen Protection**:
  - Multi-window frameless overlay with Windows Display Affinity protection (`content_protected: true`) — invisible to screen shares in Zoom, Microsoft Teams, and Google Meet where supported.
  - Instant toggle between interactive click mode and click-through invisible stealth HUD.

- 🎧 **Direct System Audio Loopback & Microphone Capture**:
  - Native Windows WASAPI loopback capture via `cpal` to transcribe interviewer speech directly from speakers/headphones with zero microphone echo.
  - Alternative direct microphone input support with automatic gain control and noise suppression.

- ⚡ **Real-Time Speech-to-Text with Failover**:
  - Primary high-accuracy transcription powered by **Deepgram Nova-3**.
  - Instant fallback to **Groq Whisper Large v3** if network errors occur.
  - Built-in Voice Activity Detection (VAD) with adaptive background noise floor calibration.

- 🧠 **Streaming AI Intelligence**:
  - Real-time streaming answers from **Google Gemini** via Server-Sent Events (SSE).
  - Markdown rendering with instant code syntax highlighting via Prism.js.
  - Tailored domain prompts for **Data Structures & Algorithms (DSA)**, **System Design**, and **Software Engineering**.

- 📸 **Instant Screen OCR & Problem Solver**:
  - Global hotkey captures the current display and feeds the screenshot directly into Gemini Multimodal for instant coding solutions.

- ❄️ **Answer Hold Mode**:
  - Freeze the current live response so new conversational background chatter does not overwrite the solution while you explain it.

---

## 🏛️ Architecture & Windows

OpenMic is split into decoupled, lightweight overlay windows running on Tauri v2:

| Window | File | Purpose |
| :--- | :--- | :--- |
| **Command Bar** | [`src/index.html`](file:///c:/Users/Admin/Desktop/openmic/src/index.html) | Sleek, draggable top bar with status indicator, mic toggle, screenshot trigger, hold badge, and quick settings. |
| **Live Answer** | [`src/live-answer.html`](file:///c:/Users/Admin/Desktop/openmic/src/live-answer.html) | Minimalistic HUD window that streams concise, real-time technical answers directly beneath the top bar. |
| **Chat Hub** | [`src/chat.html`](file:///c:/Users/Admin/Desktop/openmic/src/chat.html) | Expandable full session history, speech transcripts, and manual prompt input panel. |
| **AI Problem Solver** | [`src/llm-response.html`](file:///c:/Users/Admin/Desktop/openmic/src/llm-response.html) | Split-view inspector displaying captured screen problems and full multi-step technical solutions. |
| **Settings** | [`src/settings.html`](file:///c:/Users/Admin/Desktop/openmic/src/settings.html) | Configure API keys, speech providers, capture audio devices, active skills, and stealth presets. |

---

## ⌨️ Global Shortcuts

OpenMic can be controlled entirely via system-wide global hotkeys:

| Shortcut | Action | Description |
| :--- | :--- | :--- |
| <kbd>Alt</kbd> + <kbd>R</kbd> | **Toggle Listening** | Starts or stops audio capture and speech recognition. |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>S</kbd> | **Screen Capture** | Takes a screenshot of the active monitor and triggers problem analysis. |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>C</kbd> | **Toggle Chat & HUD** | Shows or focuses the Live Answer and Chat transcript windows. |
| <kbd>Alt</kbd> + <kbd>A</kbd> | **Click-Through Stealth** | Toggles cursor passthrough mode (makes windows click-through). |
| <kbd>Alt</kbd> + <kbd>H</kbd> | **Toggle Answer Hold** | Locks the current answer on screen so new audio does not overwrite it. |

---

## 🚀 Quick Start

### Prerequisites

- **Node.js**: `v18.0.0` or higher
- **Rust & Cargo**: Latest stable Rust toolchain ([rustup.rs](https://rustup.rs))
- **Windows C++ Build Tools**: Required for compiling Tauri v2 and native audio capture crates (`cpal`, `xcap`)

### 1. Installation

Clone the repository and install frontend dependencies:

```bash
git clone https://github.com/OmDoke/openmic.git
cd openmic
npm install
```

### 2. Configure API Keys

Copy the sample environment file:

```bash
cp .env.example .env
```

Edit `.env` with your API credentials:

```ini
# Google Gemini API Key (https://aistudio.google.com/app/apikey)
GEMINI_API_KEY="your_gemini_api_key"
GEMINI_MODEL="gemini-2.5-flash"

# Primary Speech-to-Text: Deepgram (https://console.deepgram.com)
SPEECH_PROVIDER="deepgram"
DEEPGRAM_SPEECH_KEY="your_deepgram_api_key"
DEEPGRAM_SPEECH_MODEL="nova-3"

# Fallback Speech-to-Text: Groq Whisper (https://console.groq.com)
GROQ_SPEECH_KEY="your_groq_api_key"
GROQ_SPEECH_MODEL="whisper-large-v3"

# Audio Capture Source: "system_audio" (loopback) or "microphone"
CAPTURE_SOURCE="system_audio"
```

> **Tip:** You can also enter or update keys at runtime inside the in-app **Settings** modal (`⚙`).

### 3. Run in Development Mode

```bash
npm run tauri dev
```

### 4. Build Production Executable

```bash
npm run tauri build
```

The compiled lightweight Windows binary will be generated in `src-tauri/target/release/openmic.exe`.

---

## ⚙️ Configuration Reference

| Environment Variable | Allowed Values | Default | Description |
| :--- | :--- | :--- | :--- |
| `GEMINI_API_KEY` | String | *Required* | API key from Google AI Studio. |
| `GEMINI_MODEL` | `gemini-2.5-flash`, `gemini-3.5-flash-lite`, etc. | `gemini-2.5-flash` | The Gemini model used for live answer generation. |
| `CAPTURE_SOURCE` | `system_audio`, `microphone` | `system_audio` | Capture source: `system_audio` captures meeting output; `microphone` captures your input device. |
| `DEEPGRAM_SPEECH_KEY` | String | *Recommended* | Deepgram API key for ultra-fast STT. |
| `DEEPGRAM_SPEECH_MODEL`| `nova-3`, `nova-2` | `nova-3` | Deepgram speech model. |
| `GROQ_SPEECH_KEY` | String | *Optional* | Groq API key used as automatic failover. |
| `LOG_LEVEL` | `info`, `warn`, `error` | `warn` | Logging verbosity level. |

---

## 🛠️ Prompt Customization

Custom interview persona instructions are located in the [`src/prompts/`](file:///c:/Users/Admin/Desktop/openmic/src/prompts) folder:

- [`src/prompts/dsa.md`](file:///c:/Users/Admin/Desktop/openmic/src/prompts/dsa.md): Data Structures, Algorithms, Time/Space complexity, optimal solutions.
- [`src/prompts/programming.md`](file:///c:/Users/Admin/Desktop/openmic/src/prompts/programming.md): General software development, system design, debugging, and best practices.

You can modify these files or add new markdown prompts to match your specific domain (Frontend, DevOps, ML/AI, Backend).

---

## 🔍 Troubleshooting

- **Audio not transcribing**:
  - Verify that your `DEEPGRAM_SPEECH_KEY` or `GROQ_SPEECH_KEY` is set and valid.
  - If using `system_audio`, make sure audio is actively playing through your Windows default output device.
- **Latency / Delays**:
  - Use `gemini-2.5-flash` or `gemini-3.5-flash-lite` for the fastest response latency.
  - Ensure a stable internet connection for Deepgram and Gemini streaming endpoints.
- **Window visibility in screen shares**:
  - Windows protects windows with `content_protected: true` from standard screen captures. On certain display setups or third-party capture drivers, test beforehand to ensure overlay invisibility.

---

## 📄 License

This project is licensed under the MIT License.
