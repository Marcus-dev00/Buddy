<p align="center">
  <img src="src-tauri/icons/icon.png" width="96" alt="Buddy Logo" />
</p>

<h1 align="center">Buddy</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a>
</p>

**Buddy** 是一个轻量常驻 Windows 桌面的极简灵动岛 AI 伴侣。平时它是一枚安静悬浮在屏幕边缘的黑色胶囊，陪你听歌、打盹；点开或拖入文件，随时变成你的桌面 AI 助手。

## 下载

前往 **[Releases 页面](https://github.com/Marcus-dev00/Buddy/releases/latest)** 下载最新 Windows 安装包（支持应用内自动检测与一键更新），或访问 **[Buddy 下载页](https://buddy-companion.marcus-dev00.workers.dev/)** 获取 **BuddyMobile 安卓 APK** 与在线手机端。

## Buddy 能做什么

- **灵动岛小窗 + 大窗 / 全屏模式** — 平时四边磁吸随处悬浮（`Ctrl + Alt + B` 全局呼出），一键平滑切换为居中大窗或全屏模式（左侧常驻历史边栏、支持任务栏与 Alt+Tab 自由切换）。
- **BuddyMobile 手机伴侣（局域网 / Tailscale / 免费 Cloudflare 隧道）** — 出门在外随时远程指挥电脑 Buddy 干活、审批 Agent 操作、抓拍查看电脑桌面、跨端查看 7 天截图/一键复制电脑剪贴板文字并同步定时提醒。
- **MCP 协议与 Skills 生态** — 在「设置 → MCP」中直接接入 `stdio` 或 `HTTP` MCP 服务，并在对话中通过 `/skill` 调用扩展技能。
- **实时屏幕共享协作 (`Alt + Shift + P`)** — 选定屏幕或指定窗口后展示柔和发光边框，由独立的 Watch 视觉模型按静默/均衡/积极频率轻量伴看提示，并在聊天时自动附带实时画面上下文。
- **自然神态 & 音乐律动** — 悬停时眼神跟随光标，空闲自动打盹；仅全屏游戏时沉底避让、其余场景全程置顶；播放系统音乐时随 WASAPI 节拍律动且保持对话框稳定。
- **多模型角色 & 动态思考适配** — 支持 OpenAI 兼容、Anthropic、Gemini 与本地 Ollama，支持 Chat / Watch 独立模型配对，根据所用 LLM 动态适配原生思考参数（Gemini 2.5 `thinkingBudget`、Gemini 3/3.5 `thinkingLevel`、Claude Adaptive/Extended、OpenAI `reasoning_effort`、DeepSeek/MiMo 开关或手动覆盖 `自动 / 原生 / 仅 Prompt`），并自带 `503/429` 退避重试与 `400` 降级兜底。
- **文件拖放、内置联网与桌面 Agent** — 拖入文件、图片或文件夹即可分析整理；支持内置网页搜索/抓取（带 SSRF 安全拦截），所有本地修改操作均需你确认后执行。
- **永久 Fact 记忆、长对话压缩 & 7 天记忆库 (`Alt + Shift + S`)** — 自动收录剪贴板与截图 7 天（FTS5 全文检索），支持永久 `Fact` 记忆与长对话后台滚动摘要，并提供快捷键框选截图提问。
- **模块化插件中心** — 集成 Windows 离线 OCR、桌面贴图、剪贴板智能解析、防休眠、胶囊倒计时、文件解锁 (Locksmith)、图片批量优化、PDF 工具集、Media GIF 转换及无损媒体下载 (`yt-dlp` + 共享 `FFmpeg`)。
- **本地加密隐私** — API 密钥通过 Windows DPAPI 本地加密保存，运行日志自动脱敏，聊天与记忆数据全程留在本地。
