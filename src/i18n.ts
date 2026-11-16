import type { UiLang } from "./types";

export const UI_STRINGS = {
  en: {
    collapseTitle: "Collapse (Esc)",
    captureToast: "Saved to 7-Day Memory",
    updateBanner: "Update available",
    updateNow: "Update & Restart",
    updatingPct: "Downloading",
    tabs: {
      chat: "Chat",
      memory: "7-Day Memory",
      plugins: "Plugins",
      settings: "Settings",
    },
    plugins: {
      title: "Modular Plugin Hub",
      desc: "On-demand utilities (Offline OCR, Pin to Screen, PDF Suite, FFmpeg GIF) arrive in Phase 4.",
    },
    chat: {
      backToChat: "Back",
      history: "History",
      newChat: "New Chat",
      newChatTip: "Start a New Conversation (past chats stay saved)",
      noConversations: "No past conversations yet.",
      searchHistory: "Search conversations...",
      noHistoryMatches: "No matching conversations.",
      today: "Today",
      yesterday: "Yesterday",
      earlier: "Earlier",
      messagesUnit: "msgs",
      deleteConv: "Delete Conversation",
      agentOn: "Agent",
      agentOff: "Chat Only",
      agentToggleTip: "Agent Mode is disabled in Settings (Chat Only)",
      thinkPrefix: "Think",
      thinkLevels: {
        off: { label: "Off", short: "Off", desc: "Off: direct instant reply, fastest & least strict" },
        low: { label: "Low", short: "Low", desc: "Low: quick light check before replying" },
        medium: { label: "Medium", short: "Med", desc: "Medium: balanced thinking & intent check" },
        high: { label: "High", short: "High", desc: "High: deep step-by-step verification" },
        max: { label: "Max", short: "Max", desc: "Max: simulates every outcome before answering" },
      },
      thinkingActive: "Buddy is thinking >..",
      thoughtSaved: "Buddy's thinking",
      openSettings: "Open Settings",
      capturedNow: "Captured just now",
      capturedImage: "Captured image",
      quickActions: ["Explain Code", "Translate", "Extract Text", "Summarize"],
      inputPlaceholder: "Ask Buddy anything...",
      stop: "Stop",
      send: "Send",
      workingTools: "Working on tools...",
      approve: "Approve",
      approveAll: "Approve All",
      reject: "Reject",
      approvalRequired: "Approval required",
      destructiveWarning: "Destructive / Shell action",
      rejectedLabel: "Rejected",
    },
    memory: {
      searchPlaceholder: "Search 7-Day Memory (FTS5)...",
      noMatches: "No matching memories found.",
      empty: "No captures yet — copy text or take a screenshot!",
      screenshot: "Screenshot",
      image: "Image",
      clipboard: "Clipboard",
      capturedImage: "Captured image",
      askAi: "Ask Buddy AI",
      ask: "Ask",
      copyTip: "Copy to Clipboard",
      unstarTip: "Unstar (auto-expires after 7 days)",
      starTip: "Star to keep permanently",
      deleteTip: "Delete",
    },
    settings: {
      secModel: "Model",
      secGeneral: "General",
      secActions: "Actions",
      preset: "Preset",
      protocol: "Protocol",
      baseUrl: "Base URL",
      modelId: "Model ID",
      apiKey: "API Key",
      personality: "Personality",
      editMd: "Edit .md",
      verifyHint: "Verifies API reachability before saving",
      testing: "Testing...",
      testAndSave: "Test & Save",
      saveAnyway: "Save Anyway",
      connectedSaved: "Connected & Saved",
      savedNoTest: "Saved without testing",
      language: "System Language",
      languageDesc: "Interface language (English / 中文)",
      agentMode: "Desktop Agent Mode",
      agentModeDesc: "Auto file tools & PowerShell with approval",
      thinkingLevel: "Thinking Effort",
      thinkingLevelDesc: "Depth before replying (Off -> Max)",
      workspace: "Agent Workspace",
      workspacePlaceholder: "Optional root path (default: Profile & Dropped)",
      musicReactive: "Music Reactive",
      musicReactiveDesc: "WASAPI audio rhythm & expressions",
      autoHide: "Auto-Hide in Games & Meetings",
      autoHideDesc: "Hide during fullscreen or calls",
      startup: "Launch at Startup",
      startupDesc: "Start in top capsule on Windows login",
      updatesTitle: "App Updates",
      currentVersion: "Current version",
      checkUpdates: "Check for Updates",
      checkingUpdates: "Checking...",
      upToDate: "You are on the latest version",
      updateEndpoint: "Update Feed URL (latest.json)",
      updateEndpointPlaceholder: "Optional override (defaults to tauri.conf.json)",
      resetDock: "Reset Window to Top Center",
      resetDockDesc: "Recenter Buddy at the top edge of your screen",
      resetBtn: "Reset",
      displayVsync: "Display & VSync",
      quickSnip: "Quick Snip",
      quickSnipDesc: "Hotkey: Alt + Shift + S",
      testInhale: "Test Inhale",
      testInhaleDesc: "Preview capture mouth",
      openPrompts: "Open Prompts Folder (.md)",
      openPromptsDesc: "Customize identity.md, personality.md & rules.md live",
      footerNote: "UI supports English & 中文 · AI replies match your prompt language",
    },
  },
  zh: {
    collapseTitle: "收起 (Esc)",
    captureToast: "已存入 7 天记忆",
    updateBanner: "发现新版本",
    updateNow: "立即更新并重启",
    updatingPct: "正在下载",
    tabs: {
      chat: "对话",
      memory: "7 天记忆",
      plugins: "插件",
      settings: "设置",
    },
    plugins: {
      title: "模块化插件中心",
      desc: "按需扩展工具（离线 OCR、桌面贴图、PDF 工具箱、FFmpeg 转 GIF）将在 Phase 4 推出。",
    },
    chat: {
      backToChat: "返回",
      history: "历史",
      newChat: "新对话",
      newChatTip: "开启新对话（过往记录会自动保留）",
      noConversations: "暂无历史对话。",
      searchHistory: "搜索过往对话...",
      noHistoryMatches: "没有匹配的对话。",
      today: "今天",
      yesterday: "昨天",
      earlier: "更早",
      messagesUnit: "条",
      deleteConv: "删除对话",
      agentOn: "Agent",
      agentOff: "纯对话",
      agentToggleTip: "Agent 工具已在设置中关闭（当前为纯对话）",
      thinkPrefix: "思考",
      thinkLevels: {
        off: { label: "Off", short: "Off", desc: "Off：不思考直接回答，最快但最不严谨" },
        low: { label: "Low", short: "Low", desc: "Low：快速轻度核对核心逻辑" },
        medium: { label: "Medium", short: "Med", desc: "Medium：均衡分析意图与准确性" },
        high: { label: "High", short: "High", desc: "High：深度分步推敲边界与可行性" },
        max: { label: "Max", short: "Max", desc: "Max：先预演各种结果并核验无误后再答" },
      },
      thinkingActive: "buddy正在思考>..",
      thoughtSaved: "Buddy 思考过程",
      openSettings: "打开设置",
      capturedNow: "刚刚捕获",
      capturedImage: "捕获的图片",
      quickActions: ["解释代码", "翻译", "提取文字", "总结"],
      inputPlaceholder: "说点什么，或直接拖入文件...",
      stop: "停止",
      send: "发送",
      workingTools: "正在调用工具...",
      approve: "允许",
      approveAll: "本轮全允许",
      reject: "拒绝",
      approvalRequired: "需要你确认执行",
      destructiveWarning: "删除/命令操作需单独确认",
      rejectedLabel: "已拒绝",
    },
    memory: {
      searchPlaceholder: "搜索 7 天记忆 (FTS5)...",
      noMatches: "没有找到匹配的记忆。",
      empty: "还没有捕获记录——复制文字或截图试试吧！",
      screenshot: "截图",
      image: "图片",
      clipboard: "剪贴板",
      capturedImage: "捕获的图片",
      askAi: "发给 Buddy 提问",
      ask: "问 AI",
      copyTip: "复制到剪贴板",
      unstarTip: "取消收藏（7 天后自动过期）",
      starTip: "收藏以永久保留",
      deleteTip: "删除",
    },
    settings: {
      secModel: "模型",
      secGeneral: "通用",
      secActions: "快捷操作",
      preset: "模型预设",
      protocol: "接口协议",
      baseUrl: "接口地址 (Base URL)",
      modelId: "模型 ID",
      apiKey: "API 密钥",
      personality: "性格设定",
      editMd: "编辑 .md",
      verifyHint: "保存前将先测试一次接口连通性",
      testing: "测试中...",
      testAndSave: "测试并保存",
      saveAnyway: "仍然保存",
      connectedSaved: "连接成功并已保存",
      savedNoTest: "已直接保存",
      language: "系统语言",
      languageDesc: "界面显示语言（English / 中文）",
      agentMode: "桌面 Agent 模式",
      agentModeDesc: "智能自动判断、先提问澄清并经确认后操作文件",
      thinkingLevel: "思考深度",
      thinkingLevelDesc: "选择回复前的推演深度 (Off -> Max)",
      workspace: "Agent 工作目录",
      workspacePlaceholder: "可选默认目录（留空则为用户目录与拖入路径）",
      musicReactive: "音乐律动",
      musicReactiveDesc: "WASAPI 音频节奏与曲风表情",
      autoHide: "游戏/会议自动隐藏",
      autoHideDesc: "全屏游戏或通话时自动隐藏胶囊",
      startup: "开机自启动",
      startupDesc: "登录 Windows 时静默启动",
      updatesTitle: "应用更新",
      currentVersion: "当前版本",
      checkUpdates: "检查更新",
      checkingUpdates: "正在检查...",
      upToDate: "当前已是最新版本",
      updateEndpoint: "更新源地址 (latest.json)",
      updateEndpointPlaceholder: "可选自定义地址（留空使用内置配置）",
      resetDock: "重置窗口到顶部居中",
      resetDockDesc: "将 Buddy 恢复停靠在主屏幕顶部中央",
      resetBtn: "重置",
      displayVsync: "屏幕与刷新率",
      quickSnip: "快捷截图",
      quickSnipDesc: "快捷键: Alt + Shift + S",
      testInhale: "预览吞噬动画",
      testInhaleDesc: "测试捕获张嘴动效",
      openPrompts: "打开提示词目录 (.md)",
      openPromptsDesc: "实时编辑 identity.md、personality.md 与 rules.md",
      footerNote: "界面支持 English / 中文 · AI 回复自动跟随你的对话语言",
    },
  },
} as const;

export function getStrings(lang: UiLang) {
  return UI_STRINGS[lang];
}

export function makeWelcomeGreeting(lang: UiLang): string {
  const h = new Date().getHours();
  if (lang === "zh") {
    const pool =
      h >= 5 && h < 12
        ? ["早上好，有什么可以帮忙？", "早安！今天需要我帮什么忙？", "你好，早上有什么可以帮你的？"]
        : h >= 12 && h < 18
        ? ["下午好，有什么可以帮忙？", "下午好！需要查点什么还是聊聊？", "你好，下午有什么可以帮你的？"]
        : h >= 18 && h < 23
        ? ["晚上好，有什么可以帮忙？", "晚上好！今天想聊点什么或处理什么？", "你好，有什么可以帮你的吗？"]
        : ["夜深了，有什么可以帮忙的吗？", "晚上好，有什么需要我帮忙的？", "你好，随时告诉我你需要什么。"];
    return pool[Math.floor(Math.random() * pool.length)];
  }
  const pool =
    h >= 5 && h < 12
      ? ["Good morning! What can I help with?", "Morning! How can I help today?"]
      : h >= 12 && h < 18
      ? ["Good afternoon! What can I help with?", "Afternoon! How can I help?"]
      : h >= 18 && h < 23
      ? ["Good evening! What can I help with?", "Evening! Need a hand with anything?"]
      : ["Late night! What can I help with?", "Hey there, need a hand with anything?"];
  return pool[Math.floor(Math.random() * pool.length)];
}

export function formatRelativeTime(tsMs: number, lang: UiLang): string {
  const diffSec = Math.max(1, Math.floor((Date.now() - tsMs) / 1000));
  if (diffSec < 60) return lang === "zh" ? `${diffSec}秒前` : `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return lang === "zh" ? `${diffMin}分钟前` : `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return lang === "zh" ? `${diffHr}小时前` : `${diffHr}h ago`;
  const diffDays = Math.floor(diffHr / 24);
  return lang === "zh" ? `${diffDays}天前` : `${diffDays}d ago`;
}
