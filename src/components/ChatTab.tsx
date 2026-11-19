import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Bot,
  Check,
  ChevronDown,
  Download,
  FileText,
  Film,
  History,
  ImageDown,
  Loader2,
  Lock,
  Monitor,
  Music,
  Paperclip,
  Pause,
  Pin,
  Play,
  Plus,
  ScanText,
  Shield,
  Sparkles,
  Square,
  X,
} from "lucide-react";
import { getStrings } from "../i18n";
import { ChatHistoryDrawer } from "./ChatHistoryDrawer";
import { BrowserWatchCard } from "./BrowserWatchCard";
import { AttachmentIcon, ChatMessageItem } from "./ChatMessageItem";
import { ScreenSharePanel } from "./ScreenSharePanel";
export { getStepDisplay, ThinkingSlider } from "./ChatSliders";
import type {
  AgentPermissionMode,
  ApprovalDecision,
  AttachmentPayload,
  BrowserBookmark,
  BrowserPreviewPayload,
  BrowserStatePayload,
  ChatMessage,
  ConversationSummary,
  MemoryItem,
  ModelProfile,
  PluginProgressPayload,
  ShareStatePayload,
  SkillInfo,
  ThinkingLevel,
  UiLang,
} from "../types";

interface ChatTabProps {
  lang: UiLang;
  isWindowMode?: boolean;
  agentMode: boolean;
  permissionMode: AgentPermissionMode;
  thinkingLevel: ThinkingLevel;
  activeStreamId: string | null;
  messages: ChatMessage[];
  conversations: ConversationSummary[];
  activeConversationId: number | null;
  chatInput: string;
  onChangeInput: (val: string) => void;
  onFocusInput: (focused: boolean) => void;
  isThinking: boolean;
  attachments: AttachmentPayload[];
  recentMemory: MemoryItem | null;
  onDismissRecent: () => void;
  onAddAttachment?: (att: AttachmentPayload) => void;
  onAddAttachments?: (atts: AttachmentPayload[]) => void;
  onRemoveAttachment: (idx: number) => void;
  onSend: (promptOverride?: string, attOverride?: AttachmentPayload[]) => void;
  onRetryMessage?: (assistantMsgId: number) => void;
  onEditMessage?: (userMsgId: number, newText: string) => void;
  onStopStream: () => void;
  onToggleAgentMode: () => void;
  onChangePermissionMode: (mode: AgentPermissionMode) => void;
  onResolveApproval: (stepId: string, decision: ApprovalDecision) => void;
  onNewChat: () => void;
  onSelectConversation: (id: number) => void;
  onDeleteConversation: (id: number) => void;
  onRenameConversation: (id: number, title: string) => void;
  onOpenSettings: () => void;
  enabledPlugins?: Record<string, boolean>;
  installedPlugins?: Record<string, boolean>;
  skills?: SkillInfo[];
  onOpenPlugin?: (pluginId: string, paths?: string[]) => void;
  onQuickOcr?: (source: string) => void;
  onQuickDownloadUrl?: (url: string, format: string) => void;
  pluginProgress?: Record<string, PluginProgressPayload>;
  shareState?: ShareStatePayload | null;
  browserState?: BrowserStatePayload | null;
  browserPreview?: BrowserPreviewPayload | null;
  modelProfiles?: ModelProfile[];
  activeChatProfileId?: string;
  activeModelName?: string;
  onSelectChatModel?: (profileId: string) => void;
  defaultWatchProfileId?: string;
}

export function memoryToAttachment(m: MemoryItem): AttachmentPayload {
  const isImg = m.kind === "screenshot" || m.kind === "clipboard_image";
  if (isImg) {
    return {
      kind: "image",
      label: `${m.app_name} screenshot`,
      file_path: m.file_path,
      text_content: m.content_text,
    };
  }
  return {
    kind: "text",
    label: `${m.app_name} snippet`,
    text_content: m.content_text ?? "",
  };
}

export default function ChatTab({
  lang,
  isWindowMode = false,
  agentMode,
  permissionMode,
  thinkingLevel,
  activeStreamId,
  messages,
  conversations,
  activeConversationId,
  chatInput,
  onChangeInput,
  onFocusInput,
  isThinking,
  attachments,
  recentMemory,
  onDismissRecent,
  onAddAttachment,
  onAddAttachments,
  onRemoveAttachment,
  onSend,
  onRetryMessage,
  onEditMessage,
  onStopStream,
  onToggleAgentMode,
  onChangePermissionMode,
  onResolveApproval,
  onNewChat,
  onSelectConversation,
  onDeleteConversation,
  onRenameConversation,
  onOpenSettings,
  enabledPlugins = {},
  installedPlugins = {},
  skills = [],
  onOpenPlugin,
  onQuickOcr,
  onQuickDownloadUrl,
  pluginProgress = {},
  shareState = null,
  browserState = null,
  browserPreview = null,
  modelProfiles = [],
  defaultWatchProfileId = "",
}: ChatTabProps) {
  const [showHistory, setShowHistory] = useState(false);
  const [showSharePanel, setShowSharePanel] = useState(false);
  const [showPermMenu, setShowPermMenu] = useState(false);
  const [bookmarks, setBookmarks] = useState<BrowserBookmark[]>([]);
  const [activeUserIdx, setActiveUserIdx] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [slashSelIdx, setSlashSelIdx] = useState(0);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const permMenuRef = useRef<HTMLDivElement | null>(null);
  const scrollBoxRef = useRef<HTMLDivElement | null>(null);
  const msgRefs = useRef<Record<number, HTMLDivElement | null>>({});
  const s = getStrings(lang).chat;
  const sp = getStrings(lang).plugins;

  const refreshBookmarks = useCallback(() => {
    void invoke<BrowserBookmark[]>("list_browser_bookmarks")
      .then(setBookmarks)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refreshBookmarks();
  }, [refreshBookmarks]);

  useEffect(() => {
    if (!showPermMenu) return;
    const onDocDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (permMenuRef.current && !permMenuRef.current.contains(target)) {
        setShowPermMenu(false);
      }
    };
    window.addEventListener("mousedown", onDocDown);
    return () => window.removeEventListener("mousedown", onDocDown);
  }, [showPermMenu]);

  const handleToggleBookmark = (url: string, title: string) => {
    const existing = bookmarks.find((b) => b.url === url);
    if (existing) {
      void invoke("delete_browser_bookmark", { id: existing.id })
        .then(() => setBookmarks((prev) => prev.filter((b) => b.id !== existing.id)))
        .catch(() => {});
    } else {
      void invoke<BrowserBookmark>("add_browser_bookmark", { title, url })
        .then((created) =>
          setBookmarks((prev) => [created, ...prev.filter((b) => b.id !== created.id)])
        )
        .catch(() => {});
    }
  };

  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(92, Math.max(32, el.scrollHeight))}px`;
  }, [chatInput]);

  useEffect(() => {
    const onFocusInput = () => {
      inputRef.current?.focus();
    };
    window.addEventListener("buddy-focus-chat-input", onFocusInput);
    return () => window.removeEventListener("buddy-focus-chat-input", onFocusInput);
  }, []);

  const handleOpenSharePanel = () => {
    const next = !showSharePanel;
    setShowSharePanel(next);
    if (next) {
      setShowHistory(false);
    }
  };

  const registerUserRef = useCallback((id: number, el: HTMLDivElement | null) => {
    msgRefs.current[id] = el;
  }, []);

  const enabledSkills = skills.filter((sk) => sk.enabled);
  const slashMatch = !slashDismissed ? chatInput.match(/^\/([^\s]*)$/) : null;
  const slashQuery = slashMatch ? slashMatch[1].toLowerCase() : "";
  const slashMenuOpen = Boolean(slashMatch);
  const slashMatches = slashMenuOpen
    ? enabledSkills.filter(
        (sk) =>
          !slashQuery ||
          sk.id.toLowerCase().includes(slashQuery) ||
          sk.name.toLowerCase().includes(slashQuery) ||
          sk.description.toLowerCase().includes(slashQuery)
      )
    : [];

  // Detect any `/skill-id` explicitly typed in the draft
  const activeSlashSkill = (() => {
    for (const tok of chatInput.trim().split(/\s+/)) {
      if (!tok.startsWith("/") || tok.length < 2) continue;
      const q = tok.slice(1).toLowerCase();
      const hit = enabledSkills.find(
        (sk) => sk.id.toLowerCase() === q || sk.name.toLowerCase() === q
      );
      if (hit) return hit;
    }
    return null;
  })();

  const applySlashSkill = (sk: SkillInfo) => {
    onChangeInput(`/${sk.id} `);
    setSlashDismissed(true);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  };

  const userMessages = messages.filter((m) => m.role === "user");

  useEffect(() => {
    if (!showHistory) {
      // Scroll ONLY the inner chat message container so ancestor collapsed clip/root never scrolls Buddy off-screen.
      const box = scrollBoxRef.current;
      if (box) {
        box.scrollTo({ top: box.scrollHeight, behavior: "smooth" });
      }
      if (userMessages.length > 0) {
        setActiveUserIdx(userMessages.length - 1);
      }
    }
  }, [messages.length, isThinking, showHistory]);

  const handleChatScroll = () => {
    const box = scrollBoxRef.current;
    if (!box || userMessages.length === 0) return;
    if (box.scrollHeight - box.scrollTop - box.clientHeight < 18) {
      setActiveUserIdx(userMessages.length - 1);
      return;
    }
    const anchor = box.scrollTop + box.clientHeight * 0.35;
    let best = 0;
    for (let i = 0; i < userMessages.length; i++) {
      const el = msgRefs.current[userMessages[i].id];
      if (el && el.offsetTop <= anchor) {
        best = i;
      }
    }
    setActiveUserIdx(best);
  };

  const jumpToUserMsg = (msgId: number, userIdx: number) => {
    setActiveUserIdx(userIdx);
    const el = msgRefs.current[msgId];
    const box = scrollBoxRef.current;
    if (el && box) {
      box.scrollTo({ top: Math.max(0, el.offsetTop - 8), behavior: "smooth" });
    }
  };

  type MinimapNode = {
    key: string;
    kind: "bar" | "dot";
    startIdx: number;
    endIdx: number;
    targetId: number;
    title: string;
  };

  const minimapNodes: MinimapNode[] = [];
  const fullGroups = Math.floor(userMessages.length / 3);
  for (let g = 0; g < fullGroups; g++) {
    const start = g * 3;
    const end = start + 2;
    const snippet = userMessages[start].text.slice(0, 28);
    minimapNodes.push({
      key: `bar_${g}`,
      kind: "bar",
      startIdx: start,
      endIdx: end,
      targetId: userMessages[start].id,
      title: `#${start + 1}-${end + 1}: ${snippet}`,
    });
  }
  for (let i = fullGroups * 3; i < userMessages.length; i++) {
    const snippet = userMessages[i].text.slice(0, 28);
    minimapNodes.push({
      key: `dot_${i}`,
      kind: "dot",
      startIdx: i,
      endIdx: i,
      targetId: userMessages[i].id,
      title: `#${i + 1}: ${snippet}`,
    });
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const raw = chatInput.trim();
    const m = /^\/timer\s+(.+)$/i.exec(raw);
    if (m && enabledPlugins.keep_awake_timer !== false) {
      const arg = m[1].trim().toLowerCase();
      if (arg === "stop" || arg === "cancel" || arg === "off") {
        void invoke("plugin_cancel_timer");
        onChangeInput("");
        return;
      }
      const numMatch = /^(\d+)\s*(s|sec|m|min|h|hr)?$/i.exec(arg);
      if (numMatch) {
        const val = Math.max(1, Number(numMatch[1]));
        const unit = (numMatch[2] ?? "m").toLowerCase();
        const seconds = unit.startsWith("s") ? val : unit.startsWith("h") ? val * 3600 : val * 60;
        void invoke("plugin_start_timer", { seconds, label: arg });
        onChangeInput("");
        return;
      }
    }
    onSend();
  };

  const triggerQuickPill = (action: string) => {
    if (!recentMemory || isThinking) return;
    const att = memoryToAttachment(recentMemory);
    onDismissRecent();
    onSend(action, [att]);
  };

  const activeConv = conversations.find((c) => c.id === activeConversationId);

  return (
    <div className={`flex-1 flex min-h-0 ${isWindowMode ? "flex-row" : "flex-col"}`}>
      {/* Permanent Left History Sidebar in Large Window Mode */}
      {isWindowMode && (
        <aside className="w-[268px] shrink-0 bg-[#08080b] border-r border-white/[0.08] p-3.5 flex flex-col min-h-0">
          <div className="flex items-center justify-between gap-2 pb-2.5 mb-2 border-b border-white/[0.07] shrink-0">
            <span className="text-[12px] font-medium text-white/65 flex items-center gap-1.5">
              <History className="w-3.5 h-3.5" />
              <span>
                {s.history} ({conversations.filter((c) => c.message_count > 0).length})
              </span>
            </span>
            <button
              type="button"
              onClick={() => {
                setShowSharePanel(false);
                onNewChat();
              }}
              disabled={isThinking}
              title={s.newChatTip}
              className="px-2.5 py-1 rounded-full bg-white hover:bg-white/90 text-[11px] font-semibold text-black inline-flex items-center gap-1 disabled:opacity-40 transition"
            >
              <Plus className="w-3 h-3" />
              <span>{s.newChat}</span>
            </button>
          </div>
          <ChatHistoryDrawer
            sidebar
            lang={lang}
            conversations={conversations}
            activeConversationId={activeConversationId}
            onSelectConversation={(id) => {
              setShowSharePanel(false);
              onSelectConversation(id);
            }}
            onDeleteConversation={onDeleteConversation}
            onRenameConversation={onRenameConversation}
            onClose={() => {}}
          />
        </aside>
      )}

      <div className={`flex-1 flex flex-col min-w-0 min-h-0 ${isWindowMode ? "px-6 py-3" : ""}`}>
      {/* Top Bar: Aligned to reading column in Window Mode */}
      <div
        className={`flex items-center justify-between gap-2 shrink-0 border-b border-white/[0.06] ${
          isWindowMode
            ? "max-w-3xl w-full mx-auto pb-2.5 mb-2.5"
            : "pb-1.5 mb-1.5"
        }`}
      >
        {isWindowMode ? (
          <div className="text-[13.5px] font-semibold text-white/90 truncate">
            {activeConv && activeConv.message_count > 0 ? activeConv.title : s.newChat}
          </div>
        ) : (
        <button
          onClick={() => {
            const next = !showHistory;
            setShowHistory(next);
            if (next) setShowSharePanel(false);
          }}
          className={`flex items-center gap-1 px-1.5 py-0.5 rounded-lg text-[11px] min-w-0 transition ${
            showHistory
              ? "bg-white/15 text-white"
              : "text-white/55 hover:text-white hover:bg-white/[0.07]"
          }`}
        >
          <History className="w-3 h-3 shrink-0" />
          <span className="truncate max-w-[136px]">
            {showHistory
              ? s.backToChat
              : activeConv && activeConv.message_count > 0
              ? activeConv.title
              : `${s.history} (${conversations.filter((c) => c.message_count > 0).length})`}
          </span>
        </button>
        )}

        <div className={`flex items-center shrink-0 ${isWindowMode ? "gap-2" : "gap-1"}`}>
          {agentMode && (
            <div ref={permMenuRef} className="relative">
              <button
                type="button"
                onClick={() => setShowPermMenu((v) => !v)}
                title={s.permModes[permissionMode].desc}
                className={`flex items-center gap-1.5 rounded-lg border transition ${
                  isWindowMode ? "px-2.5 py-1 text-[11.5px]" : "px-1.5 py-0.5 text-[10px]"
                } ${
                  showPermMenu
                    ? "bg-white/15 border-white/20 text-white"
                    : "border-white/[0.08] bg-white/[0.03] text-white/70 hover:text-white hover:bg-white/[0.08]"
                }`}
              >
                <Shield
                  className={`${isWindowMode ? "w-3.5 h-3.5" : "w-2.5 h-2.5"} shrink-0 ${
                    permissionMode === "ask"
                      ? "text-white/75"
                      : permissionMode === "auto"
                      ? "text-rose-300"
                      : "text-emerald-300"
                  }`}
                />
                <span className="truncate max-w-[84px] font-medium">
                  {s.permModes[permissionMode].label}
                </span>
                <ChevronDown className="w-3 h-3 opacity-60 shrink-0" />
              </button>
              {showPermMenu && (
                <div className="absolute right-0 top-full mt-1.5 w-56 rounded-xl bg-[#0d0d10]/98 border border-white/15 shadow-2xl backdrop-blur-xl py-1 z-30">
                  {(["ask", "default", "auto"] as const).map((mode) => {
                    const isCur = mode === permissionMode;
                    return (
                      <button
                        key={mode}
                        type="button"
                        onClick={() => {
                          setShowPermMenu(false);
                          if (!isCur) onChangePermissionMode(mode);
                        }}
                        className={`w-full px-3 py-2 text-left flex items-center justify-between gap-2 text-[11.5px] transition ${
                          isCur
                            ? "bg-white/[0.12] text-white"
                            : "text-white/75 hover:bg-white/[0.07] hover:text-white"
                        }`}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 font-medium">
                            <Shield
                              className={`w-3 h-3 shrink-0 ${
                                mode === "ask"
                                  ? "text-white/75"
                                  : mode === "auto"
                                  ? "text-rose-300"
                                  : "text-emerald-300"
                              }`}
                            />
                            <span className="truncate">{s.permModes[mode].label}</span>
                          </div>
                          <div className="text-[10px] text-white/45 truncate mt-0.5">
                            {s.permModes[mode].desc}
                          </div>
                        </div>
                        {isCur && <Check className="w-3.5 h-3.5 text-white shrink-0" />}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {!agentMode && (
            <button
              type="button"
              disabled={isThinking}
              onClick={onToggleAgentMode}
              title={s.agentToggleTip}
              className="flex items-center gap-1 px-2 py-0.5 rounded-full text-[10.5px] font-medium border bg-white/[0.05] border-white/10 text-white/45 hover:text-white/85 transition disabled:opacity-50"
            >
              <Bot className="w-3 h-3" />
              <span>{s.agentOff}</span>
            </button>
          )}

          <button
            type="button"
            onClick={handleOpenSharePanel}
            title={s.shareTitle}
            className={`flex items-center gap-1.5 rounded-lg border transition ${
              isWindowMode ? "px-2.5 py-1 text-[11.5px]" : "px-1.5 py-0.5 text-[10.5px]"
            } ${
              shareState?.active
                ? "bg-white/15 border-white/25 text-white"
                : showSharePanel
                ? "bg-white/15 border-white/20 text-white"
                : "border-white/[0.08] bg-white/[0.03] text-white/65 hover:text-white hover:bg-white/[0.08]"
            }`}
          >
            {shareState?.active ? (
              <span
                className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                  shareState.paused || shareState.minimized
                    ? "bg-white/45"
                    : "bg-red-500 animate-pulse"
                }`}
              />
            ) : (
              <Monitor className={isWindowMode ? "w-3.5 h-3.5 shrink-0" : "w-3 h-3 shrink-0"} />
            )}
            <span>{shareState?.active ? (shareState.paused ? s.sharePaused : s.shareLive) : s.shareBtn}</span>
          </button>

          {!isWindowMode && (
          <button
            onClick={() => {
              setShowHistory(false);
              setShowSharePanel(false);
              onNewChat();
            }}
            disabled={isThinking}
            title={s.newChatTip}
            aria-label={s.newChat}
            className="flex items-center justify-center w-6 h-6 rounded-lg text-white/75 hover:text-white hover:bg-white/[0.1] disabled:opacity-40 transition"
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
          )}
        </div>
      </div>

      {/* Screen Share Launcher & Live Controls Panel */}
      {showSharePanel ? (
        <div className={isWindowMode ? "flex-1 flex flex-col min-h-0 max-w-2xl w-full mx-auto" : "flex-1 flex flex-col min-h-0"}>
          <ScreenSharePanel
            lang={lang}
            shareState={shareState}
            modelProfiles={modelProfiles}
            defaultWatchProfileId={defaultWatchProfileId}
            onClose={() => setShowSharePanel(false)}
          />
        </div>
      ) : /* Past Conversations Drawer (compact island mode only) */ !isWindowMode && showHistory ? (
        <ChatHistoryDrawer
          lang={lang}
          conversations={conversations}
          activeConversationId={activeConversationId}
          onSelectConversation={onSelectConversation}
          onDeleteConversation={onDeleteConversation}
          onRenameConversation={onRenameConversation}
          onClose={() => setShowHistory(false)}
        />
      ) : (
        <div
          className={`flex-1 flex flex-col min-h-0 ${
            isWindowMode ? "max-w-3xl w-full mx-auto" : ""
          }`}
        >
          {browserState?.active && (
            <BrowserWatchCard
              lang={lang}
              browserState={browserState}
              preview={browserPreview}
              isThinking={isThinking}
              mediaDownloadReady={Boolean(installedPlugins.media_downloader)}
              isBookmarked={bookmarks.some((b) => b.url === browserState.url)}
              onToggleBookmark={handleToggleBookmark}
              onSummarizePage={(url, title) =>
                onSend(
                  lang === "zh"
                    ? `请读取并帮我总结当前内置浏览器打开的网页内容：${title} (${url})`
                    : `Please read and summarize the webpage currently open in the built-in browser: ${title} (${url})`
                )
              }
              onQuickDownloadUrl={onQuickDownloadUrl}
            />
          )}
          {shareState?.active && (
            <div className="mb-1.5 px-2 py-1 rounded-xl bg-white/[0.05] border border-white/15 flex items-center justify-between gap-2 shrink-0">
              <button
                type="button"
                onClick={handleOpenSharePanel}
                className="min-w-0 flex items-center gap-1.5 text-left flex-1"
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                    shareState.paused || shareState.minimized
                      ? "bg-white/40"
                      : "bg-red-500 animate-pulse"
                  }`}
                />
                <span className="text-[10px] font-semibold tracking-wide text-white/90 shrink-0">
                  {shareState.minimized
                    ? s.shareMinimized
                    : shareState.paused
                    ? s.sharePaused
                    : shareState.analyzing
                    ? s.shareAnalyzing
                    : s.shareLive}
                </span>
                <span className="text-[10.5px] text-white/60 truncate">
                  · {shareState.last_error ? shareState.last_error : shareState.target_label}
                </span>
              </button>
              <div className="flex items-center gap-1 shrink-0">
                <button
                  type="button"
                  onClick={() => void invoke("toggle_screen_share_pause").catch(() => {})}
                  title={shareState.paused ? s.shareResume : s.sharePause}
                  className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.15] text-white/80 transition"
                >
                  {shareState.paused ? (
                    <Play className="w-2.5 h-2.5" />
                  ) : (
                    <Pause className="w-2.5 h-2.5" />
                  )}
                </button>
                <button
                  type="button"
                  onClick={() => void invoke("stop_screen_share").catch(() => {})}
                  title={s.shareStop}
                  className="p-1 rounded-lg bg-rose-500/15 hover:bg-rose-500/30 text-rose-200 transition"
                >
                  <Square className="w-2.5 h-2.5 fill-current" />
                </button>
              </div>
            </div>
          )}

          {/* Messages + Turn Minimap Rail */}
          <div className="flex-1 flex min-h-0 gap-1">
            <div
              ref={scrollBoxRef}
              onScroll={handleChatScroll}
              className={`flex-1 overflow-y-auto space-y-2 pr-1 leading-relaxed select-text ${
                isWindowMode ? "text-[13.5px] space-y-3 py-1" : "text-[12.5px]"
              }`}
            >
              {(() => {
                const lastAssistant = [...messages]
                  .reverse()
                  .find((msg) => msg.role === "assistant" && msg.id !== 0);
                const lastAssistantId = lastAssistant?.id ?? -1;
                return messages.map((m) => (
                  <ChatMessageItem
                    key={m.id}
                    message={m}
                    lang={lang}
                    isLiveStream={isThinking && activeStreamId === m.id.toString()}
                    thinkingLevel={thinkingLevel}
                    canRetry={!isThinking && (Boolean(m.isError) || m.id === lastAssistantId)}
                    onRetry={onRetryMessage ? () => onRetryMessage(m.id) : undefined}
                    canEdit={!isThinking && m.role === "user"}
                    onEditMessage={
                      onEditMessage ? (newText) => onEditMessage(m.id, newText) : undefined
                    }
                    onResolveApproval={onResolveApproval}
                    onOpenSettings={onOpenSettings}
                    onRegisterUserRef={registerUserRef}
                  />
                ));
              })()}
            </div>

            {userMessages.length >= 2 && (
              <div
                className="w-3 shrink-0 flex flex-col items-center justify-center gap-1.5 py-1 overflow-y-auto [&::-webkit-scrollbar]:hidden select-none"
                aria-label="Conversation navigation"
              >
                {minimapNodes.map((node) => {
                  const isActive =
                    activeUserIdx >= node.startIdx && activeUserIdx <= node.endIdx;
                  return (
                    <button
                      key={node.key}
                      type="button"
                      title={node.title}
                      onClick={() => jumpToUserMsg(node.targetId, node.startIdx)}
                      className={`rounded-full transition-all cursor-pointer hover:scale-125 ${
                        node.kind === "bar" ? "w-[4px] h-[14px]" : "w-[5px] h-[5px]"
                      } ${
                        isActive
                          ? "bg-white/80"
                          : "bg-white/20 hover:bg-white/50"
                      }`}
                    />
                  );
                })}
              </div>
            )}
          </div>

          {/* 30s Instant Capture Card with Quick Action Pills */}
          {recentMemory && attachments.length === 0 && (
            <div className="mt-2 rounded-xl bg-white/[0.06] border border-white/10 p-2 shrink-0">
              <div className="flex items-center justify-between text-[10.5px] text-white/55 mb-1">
                <span className="flex items-center gap-1.5">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  {s.capturedNow} · {recentMemory.app_name}
                </span>
                <button onClick={onDismissRecent} className="hover:text-white">
                  <X className="w-3 h-3" />
                </button>
              </div>
              <div className="text-[11px] font-mono text-white/80 truncate mb-1.5">
                {recentMemory.content_text || s.capturedImage}
              </div>
              <div className="flex flex-wrap gap-1">
                {s.quickActions.map((action) => (
                  <button
                    key={action}
                    onClick={() => triggerQuickPill(action)}
                    className="px-2 py-0.5 rounded-full bg-white/10 hover:bg-sky-500/30 text-[10.5px] text-white/90 transition"
                  >
                    {action}
                  </button>
                ))}
                {(recentMemory.kind === "screenshot" || recentMemory.kind === "clipboard_image") &&
                  recentMemory.file_path && (
                    <>
                      {enabledPlugins.offline_ocr !== false && onQuickOcr && (
                        <button
                          type="button"
                          onClick={() => {
                            const fp = recentMemory.file_path!;
                            onDismissRecent();
                            onQuickOcr(fp);
                          }}
                          className="px-2 py-0.5 rounded-full bg-sky-500/20 hover:bg-sky-500/35 text-[10.5px] text-sky-200 flex items-center gap-1 transition"
                        >
                          <ScanText className="w-2.5 h-2.5" />
                          <span>{sp.ocrAction}</span>
                        </button>
                      )}
                      {enabledPlugins.pin_to_screen !== false && (
                        <button
                          type="button"
                          onClick={() => {
                            const fp = recentMemory.file_path!;
                            onDismissRecent();
                            void invoke("plugin_pin_image", { source: fp }).catch(() => {});
                          }}
                          className="px-2 py-0.5 rounded-full bg-amber-500/20 hover:bg-amber-500/35 text-[10.5px] text-amber-200 flex items-center gap-1 transition"
                        >
                          <Pin className="w-2.5 h-2.5" />
                          <span>{sp.pinAction}</span>
                        </button>
                      )}
                    </>
                  )}
                {recentMemory.kind === "clipboard_text" &&
                  recentMemory.content_text &&
                  enabledPlugins.clipboard_parser !== false && (
                    <button
                      type="button"
                      onClick={() => {
                        void invoke("plugin_clean_paste", {
                          text: recentMemory.content_text,
                        }).catch(() => {});
                        onDismissRecent();
                      }}
                      className="px-2 py-0.5 rounded-full bg-emerald-500/20 hover:bg-emerald-500/35 text-[10.5px] text-emerald-200 flex items-center gap-1 transition"
                    >
                      <Sparkles className="w-2.5 h-2.5" />
                      <span>{sp.cleanPaste}</span>
                    </button>
                  )}
              </div>
            </div>
          )}

          {/* Pending Multi-Attachments Strip + Drop-to-Utility Plugin Bar */}
          {attachments.length > 0 && (
            <div className="mt-1.5 space-y-1 shrink-0">
              <div className="flex items-center gap-1.5 overflow-x-auto flex-nowrap pb-0.5">
                {attachments.map((att, idx) => (
                  <div
                    key={idx}
                    title={att.file_path ?? att.label}
                    className="shrink-0 flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-sky-500/15 border border-sky-400/25 text-[10.5px] text-sky-200 max-w-[165px]"
                  >
                    <AttachmentIcon kind={att.kind} />
                    <span className="truncate font-mono">{att.label}</span>
                    <button
                      onClick={() => onRemoveAttachment(idx)}
                      className="text-white/60 hover:text-white shrink-0"
                    >
                      <X className="w-2.5 h-2.5" />
                    </button>
                  </div>
                ))}
              </div>
              {(() => {
                const filePaths = attachments
                  .map((a) => a.file_path)
                  .filter((p): p is string => Boolean(p));
                if (filePaths.length === 0) return null;
                const hasImg = filePaths.some((p) => /\.(png|jpe?g|webp|bmp)$/i.test(p));
                const hasVid = filePaths.some((p) => /\.(mp4|mov|mkv|webm|avi)$/i.test(p));
                const hasDocOrImg = hasImg || filePaths.some((p) => /\.(pdf|md|txt|docx?|pptx?|xlsx?)$/i.test(p));
                const hasSkillCand =
                  attachments.some((a) => a.kind === "folder") ||
                  filePaths.some((p) => /\.(zip)$/i.test(p) || /skill\.md$/i.test(p));
                return (
                  <div className="flex items-center gap-1 overflow-x-auto flex-nowrap pb-0.5 [&>*]:shrink-0">
                    {hasSkillCand && (
                      <button
                        type="button"
                        onClick={() => {
                          void invoke("import_local_skill", { path: filePaths[0] })
                            .then(() => {
                              if (onOpenPlugin) onOpenPlugin("__skills__", filePaths);
                            })
                            .catch(() => {});
                        }}
                        className="px-2 py-0.5 rounded-full bg-amber-500/20 hover:bg-amber-500/35 text-[10px] text-amber-200 flex items-center gap-1 transition"
                      >
                        <Sparkles className="w-2.5 h-2.5" />
                        <span>{sp.installSkillAction}</span>
                      </button>
                    )}
                    {hasImg && enabledPlugins.offline_ocr !== false && onQuickOcr && (
                      <button
                        type="button"
                        onClick={() => onQuickOcr(filePaths[0])}
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-sky-500/30 text-[10px] text-sky-200 flex items-center gap-1 transition"
                      >
                        <ScanText className="w-2.5 h-2.5" />
                        <span>{sp.ocrAction}</span>
                      </button>
                    )}
                    {hasImg && enabledPlugins.pin_to_screen !== false && (
                      <button
                        type="button"
                        onClick={() =>
                          void invoke("plugin_pin_image", { source: filePaths[0] }).catch(() => {})
                        }
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-amber-500/30 text-[10px] text-amber-200 flex items-center gap-1 transition"
                      >
                        <Pin className="w-2.5 h-2.5" />
                        <span>{sp.pinAction}</span>
                      </button>
                    )}
                    {hasImg && enabledPlugins.image_optimizer !== false && onOpenPlugin && (
                      <button
                        type="button"
                        onClick={() => onOpenPlugin("image_optimizer", filePaths)}
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-cyan-500/30 text-[10px] text-cyan-200 flex items-center gap-1 transition"
                      >
                        <ImageDown className="w-2.5 h-2.5" />
                        <span>{sp.optimizeBtn}</span>
                      </button>
                    )}
                    {hasDocOrImg && enabledPlugins.pdf_suite !== false && onOpenPlugin && (
                      <button
                        type="button"
                        onClick={() => onOpenPlugin("pdf_suite", filePaths)}
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-indigo-500/30 text-[10px] text-indigo-200 flex items-center gap-1 transition"
                      >
                        <FileText className="w-2.5 h-2.5" />
                        <span>PDF</span>
                      </button>
                    )}
                    {hasVid && installedPlugins.media_gif && onOpenPlugin && (
                      <button
                        type="button"
                        onClick={() => onOpenPlugin("media_gif", filePaths)}
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-violet-500/30 text-[10px] text-violet-200 flex items-center gap-1 transition"
                      >
                        <Film className="w-2.5 h-2.5" />
                        <span>{sp.toGif}</span>
                      </button>
                    )}
                    {enabledPlugins.file_locksmith !== false && onOpenPlugin && (
                      <button
                        type="button"
                        onClick={() => onOpenPlugin("file_locksmith", filePaths)}
                        className="px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-rose-500/30 text-[10px] text-rose-200 flex items-center gap-1 transition"
                      >
                        <Lock className="w-2.5 h-2.5" />
                        <span>{sp.checkLocks}</span>
                      </button>
                    )}
                  </div>
                );
              })()}
            </div>
          )}

          {/* Input Bar + /skill Slash Popover */}
          <div className="relative mt-2 shrink-0">
            {slashMenuOpen && (
              <div className="absolute bottom-full left-0 right-0 mb-1.5 rounded-xl bg-[#0e1017]/95 backdrop-blur-xl border border-white/15 shadow-2xl overflow-hidden z-30 max-h-48 flex flex-col">
                <div className="px-2.5 py-1.5 border-b border-white/10 flex items-center justify-between text-[10px] text-white/50">
                  <span className="flex items-center gap-1">
                    <Sparkles className="w-3 h-3 text-amber-300" />
                    <span>{s.slashSkillsTitle}</span>
                  </span>
                  {onOpenPlugin && (
                    <button
                      type="button"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onOpenPlugin("__skills__");
                      }}
                      className="text-sky-300 hover:text-sky-200 transition"
                    >
                      {s.slashManage}
                    </button>
                  )}
                </div>
                <div className="overflow-y-auto p-1 space-y-0.5">
                  {slashMatches.length === 0 ? (
                    <div className="px-2.5 py-2 text-[11px] text-white/45 text-center">
                      {s.slashNoSkills}
                    </div>
                  ) : (
                    slashMatches.map((sk, idx) => {
                      const active = idx === Math.min(slashSelIdx, slashMatches.length - 1);
                      return (
                        <button
                          key={sk.id}
                          type="button"
                          onMouseDown={(e) => {
                            e.preventDefault();
                            applySlashSkill(sk);
                          }}
                          className={`w-full text-left px-2.5 py-1.5 rounded-lg flex items-start justify-between gap-2 transition ${
                            active ? "bg-white/[0.12] text-white" : "hover:bg-white/[0.07] text-white/85"
                          }`}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-1.5">
                              <span className="font-mono text-[11.5px] font-semibold text-amber-200">
                                /{sk.id}
                              </span>
                              <span className="text-[10.5px] text-white/55 truncate">
                                {sk.name}
                              </span>
                            </div>
                            <p className="text-[10px] text-white/50 truncate mt-0.5">
                              {sk.description}
                            </p>
                          </div>
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-white/[0.07] text-white/50 shrink-0 mt-0.5">
                            {sk.source === "builtin" ? "built-in" : "skill"}
                          </span>
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            )}

            {activeSlashSkill && !slashMenuOpen && (
              <div className="mb-1 flex items-center gap-1.5 text-[10px] text-amber-200/90 px-1">
                <Sparkles className="w-2.5 h-2.5 shrink-0" />
                <span className="font-mono font-semibold">/{activeSlashSkill.id}</span>
                <span className="text-white/45 truncate">{activeSlashSkill.name}</span>
              </div>
            )}

            {!slashMenuOpen &&
              (() => {
                const dlReady =
                  Boolean(installedPlugins.media_downloader) &&
                  Boolean(enabledPlugins.media_downloader);
                const dlProg = pluginProgress.media_downloader;
                const isDlRunning = dlReady && dlProg?.stage === "running";
                const urlMatch =
                  /((?:https?|file):\/\/[^\s]+|\b(?:localhost|127\.0\.0\.1)(?::\d+)?(?:\/[^\s]*)?)/i.exec(
                    chatInput
                  );
                if (!isDlRunning && !(urlMatch && dlReady)) return null;
                if (isDlRunning) {
                  return (
                    <div className="mb-1.5 flex items-center justify-between gap-2 rounded-lg bg-emerald-500/10 border border-emerald-400/25 px-2 py-1 text-[10px] text-emerald-200">
                      <span className="flex items-center gap-1.5 truncate font-mono">
                        <Loader2 className="w-3 h-3 animate-spin shrink-0" />
                        <span className="truncate">
                          {dlProg.percent}% · {dlProg.detail}
                        </span>
                      </span>
                      <button
                        type="button"
                        onClick={() => void invoke("cancel_plugin_task")}
                        className="shrink-0 px-1.5 py-0.5 rounded bg-rose-500/30 hover:bg-rose-500/50 text-rose-100"
                      >
                        {sp.cancelBtn}
                      </button>
                    </div>
                  );
                }
                const detectedUrl = urlMatch![1];
                return (
                  <div className="mb-1.5 flex flex-wrap items-center gap-1 px-0.5">
                    {dlReady && (
                      <>
                        <span className="text-[10px] text-emerald-300/90 flex items-center gap-1 ml-0.5">
                          <Music className="w-2.5 h-2.5" />
                          <span>{sp.dlLinkDetected}</span>
                        </span>
                        {(
                          [
                            { id: "audio_best", label: sp.dlFmtAudioBest },
                            { id: "mp4_lossless", label: sp.dlFmtMp4 },
                            { id: "audio_flac", label: sp.dlFmtFlac },
                            { id: "audio_mp3", label: sp.dlFmtMp3 },
                          ] as const
                        ).map((opt) => (
                          <button
                            key={opt.id}
                            type="button"
                            onClick={() => onQuickDownloadUrl?.(detectedUrl, opt.id)}
                            className="px-2 py-0.5 rounded-full bg-emerald-500/15 hover:bg-emerald-500/30 border border-emerald-400/25 text-[10px] text-emerald-200 flex items-center gap-1 transition"
                          >
                            <Download className="w-2.5 h-2.5" />
                            <span>{opt.label}</span>
                          </button>
                        ))}
                        {onOpenPlugin && (
                          <button
                            type="button"
                            onClick={() => onOpenPlugin("media_downloader", [detectedUrl])}
                            className="px-2 py-0.5 rounded-full bg-white/[0.06] hover:bg-white/[0.14] text-[10px] text-white/65 hover:text-white transition"
                          >
                            {sp.dlInPlugins}
                          </button>
                        )}
                      </>
                    )}
                  </div>
                );
              })()}

            <form onSubmit={handleSubmit} className="flex items-end gap-1.5">
              {onAddAttachments && !isThinking && (
                <button
                  type="button"
                  title={s.attachFileTip}
                  onClick={() => {
                    void invoke<AttachmentPayload[]>("pick_chat_attachments")
                      .then((picked) => {
                        if (picked && picked.length > 0) onAddAttachments(picked);
                      })
                      .catch(() => {});
                  }}
                  className="w-8 h-8 rounded-xl bg-white/[0.06] hover:bg-white/[0.14] border border-white/12 text-white/60 hover:text-white flex items-center justify-center shrink-0 transition"
                >
                  <Paperclip className="w-3.5 h-3.5" />
                </button>
              )}
              <textarea
                ref={inputRef}
                rows={1}
                value={chatInput}
                onChange={(e) => {
                  setSlashDismissed(false);
                  setSlashSelIdx(0);
                  onChangeInput(e.target.value);
                }}
                onPaste={(e) => {
                  const items = e.clipboardData?.items;
                  if (!items || !onAddAttachment) return;
                  for (let i = 0; i < items.length; i++) {
                    const item = items[i];
                    if (item.type.startsWith("image/")) {
                      const file = item.getAsFile();
                      if (!file) continue;
                      e.preventDefault();
                      const reader = new FileReader();
                      reader.onload = () => {
                        const result = typeof reader.result === "string" ? reader.result : "";
                        if (!result) return;
                        void invoke<AttachmentPayload>("save_pasted_image_attachment", {
                          dataBase64: result,
                          mimeType: item.type,
                        })
                          .then((att) => onAddAttachment(att))
                          .catch(() => {});
                      };
                      reader.readAsDataURL(file);
                      return;
                    }
                  }
                }}
                onKeyDown={(e) => {
                  if (slashMenuOpen && slashMatches.length > 0) {
                    const safeIdx = Math.min(slashSelIdx, slashMatches.length - 1);
                    if (e.key === "ArrowDown") {
                      e.preventDefault();
                      setSlashSelIdx((safeIdx + 1) % slashMatches.length);
                      return;
                    } else if (e.key === "ArrowUp") {
                      e.preventDefault();
                      setSlashSelIdx((safeIdx - 1 + slashMatches.length) % slashMatches.length);
                      return;
                    } else if (e.key === "Tab") {
                      e.preventDefault();
                      applySlashSkill(slashMatches[safeIdx]);
                      return;
                    } else if (e.key === "Enter") {
                      const picked = slashMatches[safeIdx];
                      if (chatInput.trim().toLowerCase() !== `/${picked.id.toLowerCase()}`) {
                        e.preventDefault();
                        applySlashSkill(picked);
                        return;
                      }
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      setSlashDismissed(true);
                      return;
                    }
                  }
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    handleSubmit(e);
                  }
                }}
                onFocus={() => onFocusInput(true)}
                onBlur={() => onFocusInput(false)}
                placeholder={isThinking ? s.steerPlaceholder : s.inputPlaceholder}
                className={`flex-1 rounded-xl bg-white/[0.07] border border-white/15 focus:border-white/40 outline-none px-3 text-white placeholder:text-white/40 resize-none max-h-[120px] overflow-y-auto ${
                  isWindowMode ? "py-2 text-[13.5px] leading-[1.45]" : "py-1.5 text-[12.5px] leading-[1.4]"
                }`}
              />
              {isThinking ? (
                <div className="flex items-center gap-1.5 shrink-0">
                  {chatInput.trim().length > 0 && (
                    <button
                      type="submit"
                      className="px-2.5 py-1.5 rounded-xl bg-white hover:bg-white/90 text-black font-semibold text-[11.5px] transition"
                    >
                      {s.steerBtn}
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={onStopStream}
                    className="px-3 py-1.5 rounded-xl bg-rose-500/80 hover:bg-rose-500 text-white font-semibold text-[12px] flex items-center gap-1 transition"
                  >
                    <Square className="w-3 h-3 fill-current" />
                    <span>{s.stop}</span>
                  </button>
                </div>
              ) : (
                <button
                  type="submit"
                  className="px-3 py-1.5 rounded-xl bg-white hover:bg-white/90 text-black font-semibold text-[12px] transition"
                >
                  {s.send}
                </button>
              )}
            </form>
          </div>
        </div>
      )}
      </div>
    </div>
  );
}
