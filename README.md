<p align="center">
  <img src="src-tauri/icons/icon.png" width="96" alt="Buddy Logo" />
</p>

<h1 align="center">Buddy</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a>
</p>

**Buddy** is a minimalist desktop AI companion for Windows. It lives quietly as a floating black pill on your screen—listening to your music, napping when you step away, and expanding into an AI chat & desktop assistant whenever you click it or drop a file.

## Download

Get the latest Windows installer from the **[Releases](https://github.com/Marcus-dev00/Buddy/releases/latest)** page. Built-in auto-updates keep Buddy up to date automatically.

## What Buddy Can Do

- **Lives Anywhere on Your Screen** — Drag Buddy freely across monitors or snap it magnetically to all four screen edges (top, bottom, left, right) or target windows; its face and expanded panel orient automatically, with `Ctrl + Alt + B` to toggle anywhere.
- **Live Screen Share Co-Working (`Alt + Shift + P`)** — Share a monitor or app window with a soft perimeter border and quiet settle-debounced observations using a dedicated Watch Vision model, plus automatic multimodal context when chatting.
- **Lively Expressions & Music Rhythm** — Responds smoothly to hover gaze, naps after idle time, hides only behind fullscreen games (staying topmost elsewhere), and dances gently with WASAPI system audio without shaking your chat panel.
- **Multi-Model AI & Dynamic Thinking Depth** — Connects to OpenAI-compatible, Anthropic, and Gemini models with separate Chat vs. Watch role profiles, dynamic per-LLM thinking adaptation (Gemini 2.5 `thinkingBudget`, Gemini 3/3.5 `thinkingLevel`, Claude Adaptive/Extended, OpenAI `reasoning_effort`, DeepSeek/MiMo toggles, or manual `Auto / Native / Prompt` override), and automatic `503/429` retry + `400` downgrade protection.
- **Desktop Agent, Web Search & File Drag-and-Drop** — Drop files, images, or folders directly onto Buddy, search/fetch public web pages cleanly (with SSRF protection), or run workspace tasks with explicit approval before any changes.
- **Durable Facts, Compaction & 7-Day Memory (`Alt + Shift + S`)** — Indexes clipboard items and screenshots for 7 days (with FTS5 search), preserves permanent `Fact` memories across conversations, summarizes long transcripts cleanly, and supports instant snip capture.
- **Modular Plugin Hub** — Built-in Windows Offline OCR, Pin to Screen, Smart Clipboard Parser, Keep Awake, Capsule Timer, File Locksmith, Image Optimizer, Document & PDF Suite, Media GIF converter, and lossless Media Downloader (`yt-dlp` + shared `FFmpeg`).
- **Private & Local** — API keys are encrypted locally using Windows DPAPI, diagnostic logs (`buddy.log`) redact keys/tokens automatically, and all chat history and memories stay on your device.
