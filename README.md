# Buddy

<p align="center">
  <strong>Windows 桌面灵动岛 AI 伴侣 · 基于 Tauri v2 + Rust + React 构建</strong>
</p>

Buddy 是一个轻量常驻 Windows 桌面的黑色胶囊形态 AI 助手：平时安静停靠在屏幕边缘陪你听歌、打盹或默默收录剪贴板/截图，随时点击或拖入文件即可展开成为你的桌面 AI Agent 与对话工作台。

---

## 下载与安装 (Windows 10 / 11)

### 方式一：直接下载安装包（推荐）

前往项目的 **[Releases 页面](https://github.com/Marcus-dev00/Buddy/releases/latest)**，下载最新版本的 Windows 安装包（如 `Buddy_x.x.x_x64-setup.exe` 或 `.msi`）并双击安装。

> 首次安装完成后，后续可通过「**设置 → 通用 → 应用更新**」或顶部更新横幅直接实现应用内一键下载并重启升级。

### 方式二：从源码本地运行 / 构建

环境要求：**Node.js 20+**、**Rust (stable)**、**Windows WebView2**。

```bash
git clone https://github.com/Marcus-dev00/Buddy.git
cd Buddy
npm install
npm run tauri dev      # 开发模式运行
npm run tauri build    # 打包发布版可执行文件与安装包
```

---

## 核心功能

### 1. 灵动岛桌面交互 · 自由拖拽与四边磁吸
- **零干扰悬浮**：未展开时采用透明置顶窗口与鼠标穿透，附带 `WS_EX_NOACTIVATE`，日常敲代码、玩游戏、看视频绝不抢占前台焦点。
- **任意摆放 + 磁吸贴边**：无论是收纳态胶囊还是展开态面板，均可按住拖动到多显示器任意位置：
  - **贴顶 (`top`) / 自由悬浮 (`free`)**：头朝上，向下展开面板；
  - **贴底 (`bottom`)**：头朝上，从底边向上展开；
  - **贴左 (`left`)**：头朝向右侧，眼睛栏贴左并向右展开工作台；
  - **贴右 (`right`)**：头朝向左侧，眼睛栏贴右并向左展开工作台；
  - 靠近屏幕边缘自动亮起磁吸光条预瞄，松手即贴边吸附，并持久化记忆位置。

### 2. 拟真物理弹簧表情 & 系统音乐律动
- **自适应高刷动画引擎**：基于 `dt` 时间步长的物理弹簧与参数化 SVG 眼睛变形，自动适配屏幕原生刷新率（60Hz / 120Hz / 144Hz / 180Hz+），过渡柔和自然。
- **多状态生态**：覆盖待机打瞌睡/随机发呆/打哈欠、10 分钟无操作自动深睡（`zZ`）、悬停视线追踪（兼容左右侧旋转坐标系）、截图/复制吞噬动画与全屏自动隐身。
- **WASAPI 系统音乐律动**：实时采集系统播放音频频谱，展示音符与 4 柱跳动波形；收纳、悬停、展开态下持续保留律动状态，仅眼睛随节拍轻摆，对话窗口本身保持稳定不晃动。

### 3. 多模型 AI 对话 & 5 档思考深度
- **多协议支持**：内置 OpenAI 兼容协议、Anthropic、Gemini 配置预设与自定义 Base URL / Model ID，保存前支持连通性实测。
- **5 档思考深度 (`Off` · `Low` · `Medium` · `High` · `Max`)**：横向滑动调节思考深度，不同深度对应不同推理方式与思考眼神动效，思考过程可折叠查看。
- **高效对话管理**：支持时间分组与关键词搜索历史会话、长对话右侧快速跳转导览条（Minimap）。

### 4. 桌面 Agent & 文件/文件夹拖放
- **万物直接拖入**：把代码文件、文档、图片或整个文件夹直接拖到 Buddy 胶囊上，即可自动识别并在对话框内就绪分析。
- **受控桌面 Agent**：支持自动浏览目录、读写/搜索/重命名文件与运行命令，所有修改或高风险操作均需先由用户确认（允许一次 / 本轮允许 / 拒绝）。

### 5. 7 天本地记忆库（SQLite FTS5） & 快捷截屏
- **无感收录与全文检索**：本地监听剪贴板文本/图片与截图目录，按内容哈希自动去重并保留 7 天（支持星标永久收藏），随时搜索、复制或发送给 AI 提问。
- **快捷区域截图**：支持全局热键 **`Alt + Shift + S`** 框选屏幕内容并直接带入对话。

### 6. 本地隐私安全与实用设置
- **API Key 本地加密**：使用 Windows DPAPI 加密存储于 `%LOCALAPPDATA%/Buddy/secret.dpapi`，对话与记忆保存在本地 `buddy.db`。
- **双语切换与系统集成**：支持 **English / 中文** 界面切换、游戏/视频会议自动隐藏、Windows 开机自启动开关，以及 Tauri 官方签名自动更新。

---

## 发布新版本说明（开发者）

仓库已包含 [`.github/workflows/release.yml`](.github/workflows/release.yml)：
1. 在 GitHub 仓库 **Settings → Secrets and variables → Actions** 添加私钥 Secret：`TAURI_SIGNING_PRIVATE_KEY`（对应本地私钥文件内容，切勿提交到仓库）。
2. 更新 `src-tauri/tauri.conf.json` 与 `package.json` 中的版本号后推送 tag（如 `git tag v0.1.0 && git push origin v0.1.0`），GitHub Actions 将自动编译、签名并发布安装包与 `latest.json` 到 Releases。
