# Changelog

All notable changes to **Buddy** are documented in this file.

## [0.2.1] - 2026-11-18

### Added
- **Dynamic Per-LLM Thinking Adaptation**: Automatically adapts available thinking tiers and UI controls to the active model (`Gemini 3 / 3.5` `thinkingLevel`, `Gemini 2.5` `thinkingBudget`, `Claude 3.7–4.6` `budget_tokens`, `Claude 4.7+ / 5.x` `adaptive` + `effort`, `OpenAI` `o1/o3/o4-mini/GPT-5` `reasoning_effort`, `DeepSeek` & `Xiaomi MiMo` toggles, plus `Prompt` mode for non-reasoning/custom models), clamping saved preferences safely when switching models.
- **Manual Thinking Mode Override & Gemini 3.5 Presets**: Added per-profile **Thinking Mode** override (`Auto Detect` / `Force Native` / `Prompt Only`) in Settings alongside `Gemini 3.5 Flash Lite` and `Gemini 3.5 Flash` presets.
- **Multi-Turn Tool Thinking Continuity**: Preserved Anthropic `thinking` / `signature` / `redacted_thinking` blocks and DeepSeek/MiMo `reasoning_content` across multi-step Agent tool loops.

### Improved
- **Transient Error Resilience & Safe Downgrade**: Added exponential backoff auto-retry (`429 / 500 / 502 / 503 / 504`, timeout/connect spikes, respecting `Retry-After` and user cancellation) across streaming chat and background watch/compaction requests, plus automatic one-shot stripping/downgrade if a proxy rejects native thinking parameters (`400/422`).
- **Frontend Tab Code Splitting & Release Workflow**: Lazy-loaded `MemoryTab`, `PluginsTab`, and `SettingsTab` to bring the initial JS bundle under 500 kB, updated bilingual READMEs, upgraded GitHub Actions checkout/setup-node to Node 24 runtimes, and automated `CHANGELOG.md` extraction for GitHub Release notes.

## [0.2.0] - 2026-11-17

### Added
- **Live Screen Share Co-Working (`Alt + Shift + P` / Window & Monitor Sync)**: Added live window/monitor sharing with low-noise luma diff cadence (`Quiet`, `Balanced`, `Active`), a hollow feathered perimeter border overlay synced at 144Hz adaptive rate, quiet whisper bubbles, and automatic multimodal context when chatting (plus vision-to-text summary fallback for text-only chat models).
- **Four-Edge Magnetic Window Docking & Dedicated Role Models**: Drag Buddy onto any desktop screen edge or target window edge with automatic head orientation, plus independent Chat Model vs. Screen Watch Model selection and `model_profiles` migration.
- **Media Downloader Plugin (yt-dlp + FFmpeg)**: Added optional on-demand downloader supporting `audio_best` / `audio_flac` / `mp4_lossless` (stream copy without secondary lossy compression) and `audio_mp3` (`320kbps`), plus chat link detection and `download_media` agent tool.
- **Global Quick Toggle (`Ctrl + Alt + B`)**: Show/expand or collapse Buddy from anywhere via Win32 `RegisterHotKey` (automatically skipped while an active game triggers stealth), waking sleep mode and focusing the chat input box.
- **Native Windows File & Folder Pickers**: Added COM `IFileOpenDialog` pickers across **Settings** (Agent workspace folder) and **Plugins** (Offline OCR, Pin to Screen, Image Optimizer, PDF Suite, Media GIF, Media Downloader output folder) while preventing focus-loss auto-collapse during modal selection.
- **Rolling Diagnostic Logger**: Added sanitized operational logging (`%LOCALAPPDATA%\Buddy\logs\buddy.log`, rotating at 2 MB to `buddy.prev.log`) that automatically redacts API keys and `Bearer` tokens, plus an **Open Logs** button in Settings.
- **Durable User Memory Facts (`Fact`)**: Added permanent starred `fact` memories (exempt from 7-day automatic cleanup) with FTS5 full-text search, automatic injection of recent facts into chat context, `Fact` display/management in the Memory tab, and `save_memory_fact` / `search_memories` agent tools.
- **Rolling Conversation Summary Compaction**: Added `conversations.summary_text` and `summarized_up_to_id` schema migration plus background summarization when uncompacted turns grow long, pairing `[Earlier Conversation Summary]` with recent verbatim turns.
- **Built-in Read-Only Web Tools**: Added `web_search` and `fetch_web_page` agent tools with SSRF protection (blocking `localhost`, private/loopback/link-local IPs, metadata endpoints, internal hostnames, non-HTTP schemes, and unsafe redirects).
- **UI Polish**: Added a language header and one-click Copy button to Markdown fenced code blocks in Chat, and clarified built-in conversion vs. optional QPDF engine capabilities on the Document & PDF Suite plugin card.

### Improved
- **Strict Capsule Hitbox & Click-Through**: Fixed `WS_EX_TRANSPARENT | WS_EX_LAYERED` click-through switching without triggering `tao` frame redraw flashes, and scoped collapsed pointer handlers to `clipBoxRef` + `pillRectRef` capsule bounds so clicking transparent dialog space below the capsule passes cleanly to underlying windows instead of expanding Buddy.
- **Game-Only Stealth & Always-On-Top Elsewhere**: Removed non-game meeting/mic auto-hide triggers, excluded launchers and overlays (Steam, Wallpaper Engine, Epic Games Launcher, Riot Client, OBS, Discord, etc.) from game detection, and actively maintained `HWND_TOPMOST` in all non-game situations.
- **High-DPI Vision Clarity**: Upgraded image scaling to `FilterType::CatmullRom`, increased capture/upload quality (`2048px / q86` for captures/watch frames and `2304px / q88` for live chat frames), enabled OpenAI `"detail": "high"` image requests, and tightened screen-watch prompts to reference concrete on-screen elements.

### Known Limits & Technical Notes
- **Long-Term Memory & Conversation Compaction ("长聊天不失忆")**:
  - **Lossy Rolling Summary**: Each turn injects the rolling summary plus up to **20** recent uncompacted messages. Once uncompacted turns exceed **22**, older turns are summarized asynchronously (approx. 220-word target) while keeping the newest **12** messages verbatim. Summarization preserves key goals, paths, and decisions, rather than keeping every earlier word verbatim in LLM context (full raw messages remain viewable in the UI/SQLite DB).
  - **Model & Online Dependency**: Background compaction calls your configured Chat model after a completed turn; if the endpoint is offline, misconfigured, or returns an error, compaction is skipped safely until a later turn succeeds.
  - **Fact Injection Cap & Scope**: System prompts automatically inject the **8** most recently updated permanent `fact` entries; older facts can still be retrieved on demand via `search_memories` when **Desktop Agent Mode** is enabled. `search_memories` queries the `memories` store (Facts, Clipboard items, Screenshots), not full cross-conversation `chat_messages` transcripts.
- **Built-in Web Search & Page Fetch ("内置上网查资料")**:
  - **Agent Mode & Search Provider**: `web_search` and `fetch_web_page` require **Desktop Agent Mode** to be enabled and rely on model tool calling. `web_search` currently queries the public DuckDuckGo HTML endpoint (returning up to 5–8 results), which depends on direct network reachability to DuckDuckGo.
  - **No JavaScript Rendering or Login State**: `fetch_web_page` uses lightweight HTTP requests + HTML text extraction without a headless browser, so client-side JS/SPA pages, anti-bot gates, login-walled pages, or binary documents (e.g., direct PDF URLs) cannot be rendered or read.
  - **Size Caps & SSRF Restrictions**: Page downloads cap at **1.5 MB** raw bytes and return up to **8,000** characters by default (**16,000** max) after HTML stripping. For security, only public `http://` / `https://` targets are allowed (following at most **3** validated redirects); `localhost`, `.local`/`.internal`/`.lan`, single-label hosts, and private/loopback/link-local IPv4/IPv6 addresses are blocked.
