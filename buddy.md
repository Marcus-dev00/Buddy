# Buddy — Windows 桌面灵动岛 AI 玩伴需求与架构文档 (V1.0)

## 1. 项目概述
* **产品名称**：Buddy
* **运行平台**：Windows 10 / Windows 11
* **界面与交互语言**：UI 固定为**全英文**（Tab、按钮、设置项、系统托盘与提示文案）；AI 对话支持**多语言自适应**（用户使用任何语言提问，Buddy 均以相同语言回复与协助）。
* **核心定位**：常驻于屏幕顶部的极简灵动岛形态 AI 伴侣。融合 **“无感信息捕获（7天截图与剪贴板记忆库）”**、**“音乐律动陪伴”** 与 **“高情绪价值交互（Grok 风格发光面罩动画）”**，打造兼具极客效率与桌面陪伴感的轻量化工具。
* **本地数据目录**：`%LOCALAPPDATA%/Buddy/`（核心数据库：`buddy.db`，截图与缩略图目录：`screenshots/`、`thumbnails/`；选用 `LOCALAPPDATA` 以避免漫游配置同步大量图片缓存）

---

## 2. 视觉风格与状态机设计（Grok 极简发光面罩风）

整体采用纯黑圆角胶囊作为“面部屏幕”，五官由纯矢量发光图形（SVG + 基于 `dt` 时间步长的物理弹簧引擎）渲染，默认支持最高 180Hz 并动态自适应当前显示器原生刷新率（60Hz / 120Hz / 144Hz / 180Hz+），兼顾极致丝滑度与极低资源占用。

### 2.1 七大核心状态流转

| 状态名称 | 触发条件 | 视觉与眼睛动画表现 | 交互与性能策略 |
| :--- | :--- | :--- | :--- |
| **1. 静默待机 (Idle)** | 默认收起状态，无额外操作 | 顶部居中极简黑色胶囊。循环播放**打瞌睡**（发光眼睛半眯、眼皮下垂后突然惊醒眨眼）与**随机发呆思考**（眼珠向左上角瞟 + 极小微光 `...`）。 | **待机缩窗策略**：非展开态时直接将原生窗口收缩至胶囊尺寸，并附加 `WS_EX_NOACTIVATE` 确保永不抢占前台焦点；默认锚定主显示器顶部居中，监听 `WM_DISPLAYCHANGE` / `WM_DPICHANGED` 自动校准多屏与缩放坐标。 |
| **2. 深度熟睡 (Sleep)** | 通过 `GetLastInputInfo` 检测超过 10 分钟无键鼠操作（音乐播放时不计入），或进入深夜时段 | 双眼变为暗淡的向下弯月弧线（`◡ ◡`），右侧缓缓飘出极简像素风 `zZ` 呼吸微光。 | 显式暂停 JS 动画循环（停止 `requestAnimationFrame` / Framer Motion 连续重绘），后台 CPU 占用趋近于 0%。 |
| **3. 收录反馈 (Capture)** | 触发系统截图 或 按下 `Ctrl + C` 复制 | 胶囊纵向微张，双眼惊喜亮起，下方张开圆形发光小嘴将飞入的素材微粒“啊呜”一口吸入，随后闭嘴变为开心弯弧（`◠ ◠`）并执行向内收缩的“吞咽”弹性动画（持续约 1.5 秒）。 | 无声英文提示 `Saved to 7-Day Memory`，全程不抢焦点，动画结束后无缝返回上一状态（`Idle` 或 `Music`）。 |
| **4. 悬停唤醒 (Peek)** | 鼠标光标移至胶囊上方 | 双眼睁开并**实时追踪鼠标光标方向**，胶囊横向微微拉宽。 | 点击胶囊展开完整面板；支持直接将桌面文件/图片拖入胶囊触发问答。 |
| **5. 展开工作台 (Expanded)** | 点击胶囊 或 按下全局唤醒快捷键 | **上半区（舞台区）**：大尺寸主视觉 Grok 风格发光眼睛（自然眨眼微动）。<br>**下半区（功能区）**：主操作区 + 底部超极简微缩导航坞。 | • **聚焦/输入对话框时**：大眼睛自然垂下视线，盯着输入框看用户打字。<br>• **AI 生成回复时**：纯白大眼睛轻柔呼吸脉冲并微仰思考，回复文字以逐字打字机流式吐出。<br>• 此状态下触发复制不收起面板，直接刷新输入框上方素材卡。 |
| **6. 音乐律动 (Music)** | 检测到系统音频正在播放音乐（WASAPI Loopback + SMTC） | 眼睛与胶囊随节拍缩放脉冲，并根据识别到的音乐风格切换不同表情与微光动效（Lo-fi 眯眼摇摆、EDM 霓虹脉冲等）。 | 音频采样与 FFT 全程在 Rust 端完成，以 20~30Hz 推送平滑包络值给前端做自适应高刷（最高 180Hz+）插值渲染；人声播客/会议通话自动降级或忽略。 |
| **7. 专注隐身 (Stealth)** | 检测到全屏游戏或正在进行视频会议（默认开启） | 胶囊向上收起并**完全隐藏**（`window.hide()`），直到游戏或会议结束并防抖确认后，再从顶部边缘探头回归。 | 彻底暂停 WebView 动画与音频 FFT 轮询（游戏/会议期间 GPU/CPU 零开销）；后台剪贴板与截图监听仍可静默记录（可配置）。 |

### 2.2 状态机优先级矩阵
`Stealth (会议/游戏)` > `Expanded (工作台)` > `Capture (收录反馈)` > `Music (音乐律动)` > `Idle (待机)` > `Sleep (熟睡)`

---

## 3. 展开面板布局结构

展开后的窗口采用 **“大眼睛主舞台 + 极简功能区 + 底部微缩导航坞”** 的垂直布局（全英文 UI），尺寸严格约束为：**收起态约 `150 × 40` 逻辑像素；展开态宽度 `380 ~ 440` 逻辑像素、高度严格不超过当前屏幕可用高度的 `50%`（适配高 DPI 缩放）**：
1. **顶部主视觉眼睛区**：占据更宽裕的上方舞台空间，突出大尺寸 Grok 发光眼睛与自然微表情陪伴，不被功能按钮挤压。
2. **中部主操作区**：直接承接当前激活的功能视图。
3. **底部超极简导航坞（Micro-Dock）**：置于面板最底部的轻量化图标栏（仅当前选中项展开英文标签，其余收起为极简线框图标），避免抢占眼睛区的视觉焦点：
   * `Chat`（默认停留）：纯粹的 AI 聊天输入框与流式气泡回复区。
   * `7-Day Memory`：按时间倒序排列的截图与剪贴板混合时间轴。
   * `Plugins`：按需扩展插件中心，支持浏览、一键下载/卸载或启用/停用实用小工具，保持主程序极致轻量。
   * `Settings`：模型 API 配置、AI 性格切换、音乐律动开关、游戏/会议自动隐身开关、快捷键及开机自启设置。

---

## 4. 核心功能规格 (V1.0 MVP)

### 4.1 7天截图记忆库 (Screenshot Vault)
* **双通道无感捕获**：
  * **系统截图监听**：通过 Windows Known Folder API（`FOLDERID_Screenshots`）动态定位系统截图目录（兼容开启 OneDrive 备份后的重定向路径），自动收录 `Win + Shift + S`、`PrtSc` 等原生截图。
  * **内置快捷截图**：支持 Buddy 专属全局截图热键（基于多屏 DPI 感知的全屏透明选区），框选后自动复制进剪贴板并同步存入记忆库。
* **存储与性能优化**：
  * 原图保存至 `%LOCALAPPDATA%/Buddy/screenshots/`，同步生成轻量级缩略图至 `thumbnails/` 用于“7天记忆”列表快速滑动预览。
  * 通过 `GetForegroundWindow` 捕获当时所处的前台软件名称，连同图片路径、SHA-256 哈希与捕获时间戳一并写入 SQLite。
  * 与剪贴板图片通道共享 SHA-256 哈希去重（短时间窗口内相同图像不重复入库、不重复触发收录动画）。
* **7天生命周期管理**：
  * **应用启动时立即检查 + 后台周期性检查**，自动清理超过 7 天的记录（严格先删除磁盘原图与缩略图文件，再删除数据库记录，防止孤儿文件残留）；支持单条点击**星标锁定（Star）**以永久保留。

### 4.2 7天剪贴板记忆库 (Clipboard History)
* **零功耗事件监听**：调用 Windows 原生 `AddClipboardFormatListener` API 注册系统回调，仅在发生复制动作瞬间触发，日常待机零轮询、零 CPU 占用；读取时针对 `OpenClipboard` 偶发占用冲突（`ERROR_ACCESS_DENIED`）做短退避重试。
* **多类型支持、格式优先级与去重**：
  * 支持捕获纯文本、代码片段、URL 链接及复制的图片数据；当单次复制同时包含多种格式（如 Excel/网页的文本+图片）时，按“纯图像优先（仅图像来源） / 富文本降级为纯文本”的确定优先级提取，并设置单条内容体积上限以防止超大复制拖垮数据库。
  * 入库前计算内容 SHA-256 哈希值：连续复制相同内容时自动合并并刷新时间戳至顶部，避免列表冗余。
* **隐私过滤与检索清理**：
  * 自动忽略携带 Windows 隐私标记（`ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory`）的敏感复制内容（如密码管理器、系统密码框）。
  * 与截图库共用 7 天自动清理机制（支持星标豁免），提供顶部关键词实时过滤搜索（基于 SQLite FTS5）。

### 4.3 无缝投喂 AI 对话框 (AI Chat & Context Bridge)
* **即时素材自动挂载**：
  * 刚截图或复制内容后的 30 秒内展开面板，输入框上方自动悬浮该条素材卡片预览，并附带英文快捷指令胶囊（`Explain Code`、`Translate`、`Extract Text`、`Summarize`），点击即发（若用户已开始输入或手动关闭则隐藏，否则发送前保持可见）。
* **历史记忆回溯提问**：
  * 在 `7-Day Memory` 标签页中点击任意历史截图或文本，一键带入 `Chat` 框作为上下文进行多模态问答（支持多图同时发给模型对比；多模态图片在转 Base64 发送前会在本地按长边缩放压缩，原图仍保留于本地）。
* **可自定义大脑、人设与多语言自适应**：
  * **模型接口**：同时支持 **OpenAI Chat Completions 兼容协议**、**Anthropic Claude Messages API（`/v1/messages`）** 与 **Google Gemini 原生接口（`streamGenerateContent`）**（支持用户自由配置 Base URL、API Key、Model ID），支持流式输出与多模态识图。
  * **人设、性格与规则三层解耦（`.md` 动态加载）**：在 `%LOCALAPPDATA%/Buddy/prompts/`（源码默认模板位于 `prompts/`）下拆分为三份独立的 Markdown 文件，运行时实时读取生效、无需重编译：
    * `identity.md`（人设）：定义 Buddy 的桌面灵动岛角色身份与基础能力边界。
    * `personality.md`（性格）：通过 `## <id> | <Name>` 分段管理多种性格模板（如 `Concise Geek Assistant`、`Warm Study Companion`、`Senior Code Reviewer`），支持在 `Settings` 一键打开目录增删或编辑。
    * `rules.md`（规则）：约束多语言自适应镜像规则、准确性与回复排版规范。
* **隐私、零遥测与开箱即用**：
  * 所有 7 天截图与剪贴板历史 100% 存储于用户本地电脑，**零后台遥测（Zero Telemetry）**，绝不静默上传；仅当用户主动触发 AI 提问时，才将选定内容发送至配置的模型接口。
  * 仓库与安装包内**不硬编码任何默认 API Key**：未配置 Key 时，本地记忆、音乐律动、专注隐身及离线插件均可直接开箱即用；`Chat` 页面首次发送时引导用户在 `Settings` 填入自己的 Base URL 与 Key。
  * 模型 API Key 通过 Windows 凭据管理器（Credential Manager）或 DPAPI 加密存储，不明文保存在 `buddy.db` 或配置文件中。

### 4.4 音乐律动与风格感知 (Music Reactive & Style Sensing)
* **无感系统音频捕获（WASAPI Loopback + SMTC）**：
  * 不使用麦克风（避免环境噪音与隐私红点），直接通过 Windows **WASAPI Loopback** 捕获系统音频输出流，并结合 Windows **SMTC（`GlobalSystemMediaTransportControls`）** 识别当前是否有媒体播放及曲目元数据。
* **轻量级声学风格识别（零本地 AI 模型）**：
  * 在 Rust 端对音频流做实时 FFT，提取 **BPM 节拍、低频重低音能量（Bass）、频谱质心（亮度）与节奏稳定性**，以 <1% CPU 开销区分音乐风格并驱动不同眼睛与胶囊动效：
    * **Lo-fi / Classical / Ambient**（低 BPM、低冲击）：半眯惬意弯眼（`˘ ˘`）、温柔左右轻晃、飘出微光音符 `♪`。
    * **Pop / Indie**（中速节拍）：眼睛随鼓点轻快眨动与柔和缩放。
    * **EDM / Hip-Hop / Rock**（高 BPM / 强 Bass 冲击）：眼睛切换为高亮脉冲环/霓虹波纹，胶囊随重低音做强弹性缩放。
    * **人声播客 / 非音乐视频**：通过节拍稳定性与人声频段特征识别，降级为轻微倾听或保持待机，避免误触发蹦迪。

### 4.5 专注隐身模式 (Stealth Mode for Games & Meetings)
* **精准场景识别**：
  * **游戏与全屏应用**：调用 Windows `SHQueryUserNotificationState`（检测 `QUNS_RUNNING_D3D_FULL_SCREEN`、`QUNS_BUSY`、`QUNS_PRESENTATION_MODE`）结合前台全屏窗口判定。
  * **视频会议**：检测主流会议进程（如 Zoom、Teams、腾讯会议 `wemeetapp.exe`、飞书、钉钉、Discord、OBS 等）且同时处于**麦克风或摄像头占用状态**（读取 Windows `CapabilityAccessManager` 权限使用状态或 WASAPI 音频采集会话），避免后台挂着聊天软件时误隐身。
* **隐身与回归行为**：
  * 触发后立即隐藏胶囊窗口（`window.hide()`），完全停止渲染与音频分析；
  * 当游戏退出或会议结束后，经过 **2~3 秒防抖确认**（防止切屏瞬间闪烁），Buddy 从屏幕顶部边缘探头回归，恢复 `Idle` 或 `Music` 状态；
  * 支持在 `Settings` 与系统托盘随时开关 `Auto-Hide in Games & Meetings`（默认开启）。

### 4.6 按需插件中心 (Modular Plugin Hub)
* **核心设计原则**：核心安装包保持极致轻量，所有非核心高频 Windows 痛点工具统一收纳在 `Plugins` 页面中，由用户**自主选择下载与启用**。已启用的插件平时不堆砌界面按钮，而是通过**“文件拖入胶囊（Drop-to-Utility）”**或**“剪贴板/截图悬浮卡微动作”**按上下文动态触发：
* **轻量原生插件（零依赖 / 极小体积，一键启用）**：
  * **`Offline OCR`**：调用 Windows 原生 `Windows.Media.Ocr` API，纯离线毫秒级提取截图文字与代码，不消耗 AI Token。
  * **`Pin to Screen`**：将任意截图或图片钉在桌面最顶层作为可缩放、可调透明度的参考浮窗（底层采用 **Rust 原生 Win32 Layered Window** 直接渲染位图，不开额外 WebView2 实例，单贴图仅占数 MB 内存）。
  * **`Smart Clipboard Parser`**：离线实时解析 Unix 时间戳、JSON、Base64、HEX 色值，并提供一键 `Clean Paste`（去除 PDF 换行与富文本样式）。
  * **`File Locksmith`**：调用 Windows `RmGetList` API，将删不掉的文件拖入胶囊即可查看占用进程并一键解锁（注：受 Windows UIPI 拖放隔离限制，Buddy 必须以普通用户权限运行，遇高权限/系统进程占用时给出明确提示）。
  * **`Color Picker`**：全局屏幕像素放大镜吸管，一键复制 `HEX` / `RGB` / `HSL`。
  * **`Keep Awake & Capsule Timer`**：基于 `SetThreadExecutionState` 的防锁屏休眠开关，以及胶囊边缘微光进度环倒计时（如 `/timer 25m`）。
* **按需下载引擎插件（下载至 `%LOCALAPPDATA%/Buddy/plugins/`，支持一键卸载清理）**：
  * **`Document & PDF Suite`**：支持 Word / PPT 转 PDF（优先调用本机 Office COM；未安装 Office 时提示按需下载轻量开源转换引擎或仅使用内置的图片/Markdown 转 PDF）、PDF 合并/拆分/压缩。
  * **`Image Optimizer`**：拖入图片批量压缩、WebP/PNG/JPG 格式互转、清除照片 EXIF 隐私信息。
  * **`Media / GIF Converter`**：按需下载独立 `ffmpeg` 二进制引擎（与主程序协议隔离），拖入录屏/视频即可一键转轻量化 GIF、提取音频或压缩体积。
* **任务反馈、安全与清单管理**：
  * **耗时任务体验**：执行转换等耗时操作时，胶囊边缘显示进度微光并支持取消；输出文件默认保存至原文件同级目录（重名自动追加 `_1` 序号防覆盖），完成后提供 `Open Folder` 快捷跳转。
  * **Plugin Manifest 与校验**：通过轻量插件清单管理版本、SHA-256 校验值与可配置镜像下载源，下载中断自动清理残余临时文件，支持在 `Plugins` 页面查看占用空间并随时彻底卸载。

---

## 5. 技术栈与工程结构

* **桌面端框架**：Tauri v2（集成 `tauri-plugin-single-instance` 单实例锁、`tauri-plugin-autostart`、`tauri-plugin-updater` 自动更新，兼顾极小安装包体积与原生 Windows 窗口控制力）
* **前端渲染层**：React + TypeScript + Tailwind CSS + Framer Motion（全英文 UI、Grok 眼睛矢量动画、自适应显示器刷新率最高 180Hz+ 音乐律动与物理弹簧插值、流式 Markdown 渲染）
* **系统原生层**：Rust（以普通用户权限运行以保障 UIPI 拖放兼容；负责无焦点缩窗/展开、全局快捷键、`AddClipboardFormatListener`、`FOLDERID_Screenshots` 监听、WASAPI Loopback + SMTC 音频分析、会议/游戏隐身检测、`GetLastInputInfo` 睡眠检测、多屏 DPI 校准、按需插件下载/SHA-256 校验及安全退出清理）
* **本地持久化**：SQLite（`buddy.db`，核心包含统一 `memories` 时间轴表 + `FTS5` 全文检索、`conversations` 与 `messages` 多轮对话表，以及插件启用/本地路径状态）
* **生命周期与安全退出**：点击面板外部或按 `Esc` 仅收起回胶囊；仅通过系统托盘 `Quit` 退出，退出时依次恢复 `Keep Awake` 电源状态、注销全局热键与剪贴板监听、关闭 WASAPI 音频线程并执行 SQLite WAL `checkpoint`。

---

## 6. 分阶段开发路线图

* **Phase 1：骨架与灵魂（UI、状态机与专注隐身）**
  * 初始化 Tauri v2 + React 项目，配置单实例锁、顶部居中无边框透明置顶（`AlwaysOnTop` + `WS_EX_NOACTIVATE`）窗口，实现待机缩窗/展开动态尺寸切换及多屏 DPI 坐标校准（`WM_DISPLAYCHANGE` / `WM_DPICHANGED`）。
  * 实现 Grok 风格发光眼睛组件与全英文 UI 框架，跑通 `Idle`、`Sleep`（`GetLastInputInfo` + 暂停动画循环）、`Capture` 吞咽、`Peek` 追踪鼠标、`Expanded` 低头偷看与思考星环状态机。
  * 实现 **Stealth 专注隐身**：接入全屏游戏（`SHQueryUserNotificationState`）与视频会议（进程 + 麦克风/摄像头占用）检测，跑通自动隐身与防抖探头回归。
* **Phase 2：本地记忆中枢与音乐律动（存储、监听与声学）**
  * 接入 Rust 端 `AddClipboardFormatListener`（含重试、格式优先级与隐私标记过滤）、`FOLDERID_Screenshots` 截图目录 `notify` 监听及 `GetForegroundWindow` 前台应用识别。
  * 搭建本地 SQLite 数据库（`buddy.db` + `FTS5`），完成跨通道 SHA-256 去重、`7-Day Memory` 混合时间轴展示、搜索过滤、星标锁定及启动/周期性 7 天自动清理任务。
  * 实现多屏 DPI 感知的内置快捷截图框选热键。
  * 接入 **WASAPI Loopback + SMTC**：在 Rust 端实现实时 FFT 节拍与风格分类，前端完成 Lo-fi / Pop / EDM 等音乐律动表情与动画。
* **Phase 3：大脑接入与联调（AI 与工作流）**
  * 打通标准 OpenAI 格式流式对话接口，完成 **Xiaomi MiMo 2.6 Flash** 联调、多模态图片压缩与 Base64 投喂、30 秒即时素材悬浮卡与英文快捷指令胶囊、胶囊文件/图片拖放问答，并验证**全英文 UI 下的多语言自适应回复**与无 API Key 时的本地功能开箱可用体验。
  * 完善 `Settings` 面板（自定义 API、Credential Manager/DPAPI 凭据加密、人设切换、音乐律动与自动隐身开关、快捷键绑定、开机自启）、系统托盘控制菜单与 Clean Shutdown 退出清理。
* **Phase 4：按需插件中心（Modular Plugin Hub）**
  * 搭建 `Plugins` 页面与 Rust 端插件管理器（Plugin Manifest 版本管理、下载进度推送、SHA-256 校验、镜像源配置、目录清理与启用状态持久化）。
  * 接入轻量原生插件（`Offline OCR`、Win32 Layered Window 版 `Pin to Screen`、`Smart Clipboard Parser`、`File Locksmith`、`Color Picker`、`Keep Awake & Capsule Timer`）及按需下载引擎插件（`Document & PDF Suite`、`Image Optimizer`、`Media / GIF Converter`），并与胶囊拖放/进度反馈联动。
* **Phase 5：GitHub 公开发布与自动更新**
  * 配置 GitHub Actions 自动构建 Windows NSIS/MSI 安装包（内嵌 WebView2 引导安装器，兼容旧版 Win10）。
  * 接入 `tauri-plugin-updater` 对接 GitHub Releases（`latest.json` + 公私钥签名校验更新包），完成开源协议（MIT/Apache-2.0）、`PRIVACY.md` 及代码签名（如 SignPath.io）配置以降低 SmartScreen 误报拦截。

---

## 7. 开源分发与 GitHub 发布规范

* **安装包与运行环境兼容**：通过 GitHub Actions CI 自动构建并发布至 GitHub Releases，安装包内嵌 WebView2 Bootstrapper，确保未预装 WebView2 Runtime 的 Windows 10 设备也能顺畅安装。
* **自动更新**：集成 `tauri-plugin-updater`，启动及后台低频检查 GitHub Releases 最新版本，强制校验更新包签名。
* **代码签名与 SmartScreen**：开源仓库可申请 **SignPath.io** 等开源免费代码签名服务，减少普通用户下载运行时的 Windows Defender SmartScreen 拦截提示。
* **许可隔离与隐私透明**：主仓库采用宽松开源协议（MIT / Apache-2.0），第三方独立二进制（如 `ffmpeg`）通过 `Plugins` 按需拉取以规避 GPL 协议传染；仓库明确标注零遥测与 100% 本地存储原则。