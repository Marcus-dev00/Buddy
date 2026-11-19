import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Bell,
  Clock,
  Download,
  Loader2,
  Maximize2,
  MessageSquare,
  Minimize2,
  PanelTopClose,
  Puzzle,
  Settings,
  X,
} from "lucide-react";
import { BuddyPillFace } from "./components/BuddyPillFace";
import ChatTab, { memoryToAttachment } from "./components/ChatTab";
import {
  FloatingBubbleOverlay,
  type FloatingBubble,
} from "./components/FloatingBubbleOverlay";
import { useBuddyEvents } from "./hooks/useBuddyEvents";
import { isSideDock, useBuddyMotion } from "./hooks/useBuddyMotion";
const MemoryTab = lazy(() => import("./components/MemoryTab"));
const PluginsTab = lazy(() => import("./components/PluginsTab"));
const SettingsTab = lazy(() => import("./components/SettingsTab"));
import { getStrings, makeWelcomeGreeting } from "./i18n";
import {
  clampThinkingLevel,
  getModelThinkingCapability,
  normalizeThinkingLevel,
  resolveActiveChatModel,
} from "./thinking";
import type {
  AgentPermissionMode,
  AgentStep,
  AppSettings,
  ApprovalDecision,
  AttachmentPayload,
  BrowserPreviewPayload,
  BrowserStatePayload,
  BuddyState,
  ChatMessage,
  ColorPickedPayload,
  ConversationSummary,
  DockEdge,
  DockStatePayload,
  LayoutDims,
  MemoryItem,
  MusicStyle,
  OcrResult,
  PluginListResponse,
  PluginProgressPayload,
  PluginToolOutput,
  ReminderDuePayload,
  ShareStatePayload,
  SkillInfo,
  TabId,
  ThinkingLevel,
  TimerTickPayload,
  UiLang,
  UpdateInfo,
} from "./types";

interface StoredChatRow {
  id: number;
  role: "user" | "assistant";
  content: string;
  attachments_json: string | null;
  agent_steps_json: string | null;
  thought_text: string | null;
}

function makeWelcomeMessage(lang: UiLang): ChatMessage {
  return {
    id: 1,
    role: "assistant",
    text: makeWelcomeGreeting(lang),
  };
}

export default function App() {
  const [buddyState, setBuddyState] = useState<BuddyState>("idle");
  const [isWindowMode, setIsWindowMode] = useState(false);
  const [isWindowMaximized, setIsWindowMaximized] = useState(false);
  const [windowAnimPhase, setWindowAnimPhase] = useState<
    "idle" | "to-window" | "leaving-island" | "leaving-pill" | "island-in" | "max-pulse"
  >("idle");
  const [activeTab, setActiveTab] = useState<TabId>("chat");
  const [settings, setSettings] = useState<AppSettings>({
    api_provider: "openai",
    base_url: "https://api.xiaomimimo.com/v1",
    model_id: "mimo-v2-flash",
    persona_id: "warm_companion",
    auto_hide_stealth: false,
    music_reactive: true,
    launch_at_startup: false,
    has_api_key: false,
    agent_mode: true,
    thinking_level: "medium",
    agent_workspace: "",
    ui_language: "en",
    auto_collapse_on_blur: false,
    agent_permission_mode: "default",
  });
  const [layoutDims, setLayoutDims] = useState<LayoutDims>({
    v_w: 420,
    v_h: 460,
    h_w: 456,
    h_h: 460,
    canvas_w: 472,
    canvas_h: 476,
    pad: 8,
  });
  const [dockEdge, setDockEdge] = useState<DockEdge>("top");
  const [snapPreview, setSnapPreview] = useState<DockEdge | null>(null);
  const [appVersion, setAppVersion] = useState<string>("0.1.0");
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [pluginsData, setPluginsData] = useState<PluginListResponse>({
    plugins: [],
    mirror_prefix: "",
    total_size_bytes: 0,
  });
  const [pluginProgress, setPluginProgress] = useState<Record<string, PluginProgressPayload>>({});
  const [lastPickedColor, setLastPickedColor] = useState<ColorPickedPayload | null>(null);
  const [timerState, setTimerState] = useState<TimerTickPayload | null>(null);
  const [shareState, setShareState] = useState<ShareStatePayload | null>(null);
  const [floatingBubble, setFloatingBubble] = useState<FloatingBubble | null>(null);
  const [dueReminderBanner, setDueReminderBanner] = useState<ReminderDuePayload | null>(null);
  const [bubbleVisible, setBubbleVisible] = useState(false);
  const [whisperHovered, setWhisperHovered] = useState(false);
  const [droppedPathsHint, setDroppedPathsHint] = useState<string[]>([]);
  const [focusedPluginId, setFocusedPluginId] = useState<string | null>(null);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [refreshHz, setRefreshHz] = useState<number>(60);
  const [memoryRefresh, setMemoryRefresh] = useState(0);
  const [attachments, setAttachments] = useState<AttachmentPayload[]>([]);
  const [recentMemory, setRecentMemory] = useState<MemoryItem | null>(null);

  const [messages, setMessages] = useState<ChatMessage[]>(() => [makeWelcomeMessage("en")]);
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [activeConversationId, setActiveConversationId] = useState<number | null>(null);
  const [chatInput, setChatInput] = useState("");
  const [isThinking, setIsThinking] = useState(false);
  const [activeStreamId, setActiveStreamId] = useState<string | null>(null);
  const [browserState, setBrowserState] = useState<BrowserStatePayload | null>(null);
  const [browserPreview, setBrowserPreview] = useState<BrowserPreviewPayload | null>(null);

  const stateRef = useRef<BuddyState>("idle");
  const shareStateRef = useRef<ShareStatePayload | null>(null);
  const whisperRef = useRef<HTMLDivElement | null>(null);
  const whisperTimerRef = useRef<number | null>(null);
  const bubbleExitTimerRef = useRef<number | null>(null);
  const floatingBubbleRef = useRef<FloatingBubble | null>(null);
  const activeConversationIdRef = useRef<number | null>(null);
  const focusedRef = useRef(false);
  const thinkingRef = useRef(false);
  const thinkingLevelRef = useRef<ThinkingLevel>("medium");
  const langRef = useRef<UiLang>("en");
  const activeStreamIdRef = useRef<string | null>(null);
  const typewriterRef = useRef<{
    streamId: string | null;
    chars: string[];
    shownLen: number;
    streamDone: boolean;
    acc: number;
    lastTick: number;
  }>({
    streamId: null,
    chars: [],
    shownLen: 0,
    streamDone: false,
    acc: 0,
    lastTick: 0,
  });
  const layoutDimsRef = useRef<LayoutDims>({
    v_w: 420,
    v_h: 460,
    h_w: 456,
    h_h: 460,
    canvas_w: 472,
    canvas_h: 476,
    pad: 8,
  });
  const dockEdgeRef = useRef<DockEdge>("top");
  const snapPreviewRef = useRef<DockEdge | null>(null);
  const isDraggingRef = useRef(false);
  const dragStepInFlightRef = useRef(false);
  const dragMetaRef = useRef<{
    active: boolean;
    pointerId: number;
    startX: number;
    startY: number;
    moved: boolean;
  } | null>(null);
  const activeTabRef = useRef<TabId>("chat");
  const peekTargetRef = useRef({ dx: 0, dy: 0 });
  const captureTimerRef = useRef<number | null>(null);
  const recentTimerRef = useRef<number | null>(null);

  const musicRef = useRef<{
    playing: boolean;
    style: MusicStyle;
    bass: number;
    mid: number;
    high: number;
    beatImpulse: number;
    renderedStyle: MusicStyle;
    eqHeights: number[];
    phase: number;
  }>({
    playing: false,
    style: "lofi",
    bass: 0,
    mid: 0,
    high: 0,
    beatImpulse: 0,
    renderedStyle: "lofi",
    eqHeights: [4, 6, 8, 5],
    phase: 0,
  });

  const cardFrameRef = useRef<HTMLDivElement | null>(null);

  const switchTab = (tab: TabId) => {
    activeTabRef.current = tab;
    setActiveTab(tab);
    try {
      localStorage.setItem("buddy_active_tab", tab);
    } catch {
      // ignore
    }
  };

  const presentBubble = (next: FloatingBubble) => {
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
      bubbleExitTimerRef.current = null;
    }
    floatingBubbleRef.current = next;
    setBubbleVisible(false);
    setFloatingBubble(next);
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        setBubbleVisible(true);
      });
    });
  };

  const dismissBubbleAnimated = () => {
    if (whisperTimerRef.current) {
      window.clearTimeout(whisperTimerRef.current);
      whisperTimerRef.current = null;
    }
    setBubbleVisible(false);
    setWhisperHovered(false);
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
    }
    bubbleExitTimerRef.current = window.setTimeout(() => {
      bubbleExitTimerRef.current = null;
      floatingBubbleRef.current = null;
      setFloatingBubble(null);
    }, 220);
  };

  const {
    refs: {
      morphLayerRef,
      pillShadowRef,
      pillRectRef,
      clipBoxRef,
      eyesStageRef,
      facePivotRef,
      eyesPairRef,
      leftEyeShapeRef,
      rightEyeShapeRef,
      leftEyeGlowRef,
      rightEyeGlowRef,
      leftEyePillRef,
      rightEyePillRef,
      leftEyeGlintRef,
      rightEyeGlintRef,
      leftEyeArcRef,
      rightEyeArcRef,
      mouthSvgRef,
      mouthShapeRef,
      particlesRef,
      particleElsRef,
      sleepZRef,
      idleDotsRef,
      musicNoteRef,
      musicEqRef,
      eqBarRefs,
      captureToastRef,
    },
    springRef,
    renderEyeSvg,
    restStateOrMusic,
    applyVisualTargets,
    transitionTo,
    triggerCapture,
  } = useBuddyMotion({
    setBuddyState,
    stateRef,
    dockEdge,
    dockEdgeRef,
    layoutDims,
    setLayoutDims,
    layoutDimsRef,
    thinkingRef,
    thinkingLevelRef,
    focusedRef,
    activeTabRef,
    isDraggingRef,
    snapPreviewRef,
    peekTargetRef,
    shareStateRef,
    musicRef,
    captureTimerRef,
    typewriterRef,
    activeStreamIdRef,
    switchTab,
    setRefreshHz,
    onTypewriterFlush: (streamId, visibleText) => {
      setMessages((prev) =>
        prev.map((m) => (m.id.toString() === streamId ? { ...m, text: visibleText } : m))
      );
    },
    onTypewriterComplete: () => {
      setIsThinking(false);
      setActiveStreamId(null);
    },
  });

  const loadConversationMessages = async (convId: number, langOverride?: UiLang) => {
    activeConversationIdRef.current = convId;
    setActiveConversationId(convId);
    const curLang: UiLang = langOverride ?? (settings.ui_language === "zh" ? "zh" : "en");
    try {
      const rows = await invoke<StoredChatRow[]>("list_chat_history", {
        conversationId: convId,
      });
      if (rows.length > 0) {
        setMessages(
          rows.map((r) => ({
            id: r.id,
            role: r.role,
            text: r.content,
            thought: r.thought_text ?? undefined,
            attachments: r.attachments_json
              ? (JSON.parse(r.attachments_json) as AttachmentPayload[])
              : undefined,
            steps: r.agent_steps_json
              ? (JSON.parse(r.agent_steps_json) as AgentStep[])
              : undefined,
          }))
        );
      } else {
        setMessages([makeWelcomeMessage(curLang)]);
      }
    } catch {
      setMessages([makeWelcomeMessage(curLang)]);
    }
  };

  const refreshConversations = async () => {
    try {
      const list = await invoke<ConversationSummary[]>("list_conversations");
      setConversations(list);
    } catch {
      // ignore
    }
  };

  useBuddyEvents({
    stateRef,
    dockEdgeRef,
    snapPreviewRef,
    layoutDimsRef,
    activeTabRef,
    peekTargetRef,
    musicRef,
    recentTimerRef,
    activeStreamIdRef,
    typewriterRef,
    thinkingRef,
    langRef,
    shareStateRef,
    activeConversationIdRef,
    floatingBubbleRef,
    springRef,
    setLayoutDims,
    setDockEdge,
    setSnapPreview,
    setAppVersion,
    setPluginsData,
    setSkills,
    setShareState,
    setBrowserState,
    setBrowserPreview,
    setSettings,
    setMemoryRefresh,
    setRecentMemory,
    setAttachments,
    setDroppedPathsHint,
    setActiveTab,
    setMessages,
    setIsThinking,
    setActiveStreamId,
    setAvailableUpdate,
    setUpdateProgress,
    setPluginProgress,
    setLastPickedColor,
    setFocusedPluginId,
    setTimerState,
    loadConversationMessages,
    refreshConversations,
    applyVisualTargets,
    transitionTo,
    restStateOrMusic,
    triggerCapture,
    renderEyeSvg,
    presentBubble,
    dismissBubbleAnimated,
  });

  const handleSendChat = async (
    promptOverride?: string,
    attOverride?: AttachmentPayload[],
    isRetry = false
  ) => {
    const rawPrompt = (promptOverride ?? chatInput).trim();
    const usedAtts = attOverride ?? attachments;
    if (!rawPrompt && usedAtts.length === 0) return;

    if (isThinking) {
      if (!rawPrompt) return;
      const cid = activeConversationIdRef.current ?? 1;
      if (!promptOverride && !isRetry) setChatInput("");
      const steerMsgId = Date.now();
      const sid = activeStreamIdRef.current;
      setMessages((prev) => {
        const nextMsg: ChatMessage = { id: steerMsgId, role: "user", text: rawPrompt };
        if (!sid) return [...prev, nextMsg];
        const idx = prev.findIndex((m) => m.id.toString() === sid);
        if (idx === -1) return [...prev, nextMsg];
        const copy = prev.slice();
        copy.splice(idx, 0, nextMsg);
        return copy;
      });
      void invoke("steer_chat_stream", { conversationId: cid, prompt: rawPrompt }).catch(() => {});
      return;
    }

    let convId = activeConversationIdRef.current;
    if (convId === null) {
      try {
        convId = await invoke<number>("ensure_active_conversation");
        activeConversationIdRef.current = convId;
        setActiveConversationId(convId);
      } catch {
        convId = 1;
      }
    }

    const finalPrompt = rawPrompt || "Analyze attached item";
    if (!promptOverride && !isRetry) setChatInput("");
    if (!attOverride && !isRetry) setAttachments([]);

    const userMsgId = Date.now();
    const streamId = (userMsgId + 1).toString();
    activeStreamIdRef.current = streamId;
    typewriterRef.current = {
      streamId,
      chars: [],
      shownLen: 0,
      streamDone: false,
      acc: 0,
      lastTick: performance.now(),
    };
    thinkingRef.current = true;
    setIsThinking(true);
    setActiveStreamId(streamId);
    applyVisualTargets("expanded", true);

    setMessages((prev) =>
      isRetry
        ? [...prev, { id: userMsgId + 1, role: "assistant", text: "" }]
        : [
            ...prev,
            { id: userMsgId, role: "user", text: finalPrompt, attachments: usedAtts },
            { id: userMsgId + 1, role: "assistant", text: "" },
          ]
    );

    void invoke("send_chat_message", {
      conversationId: convId,
      streamId,
      prompt: finalPrompt,
      attachments: usedAtts,
      isRetry,
    })
      .then(() => refreshConversations())
      .catch((err) => {
        activeStreamIdRef.current = null;
        typewriterRef.current.streamId = null;
        thinkingRef.current = false;
        setIsThinking(false);
        setActiveStreamId(null);
        applyVisualTargets("expanded", false);
        setMessages((prev) =>
          prev.map((m) =>
            m.id.toString() === streamId ? { ...m, text: String(err), isError: true } : m
          )
        );
      });
  };

  const handleRetryMessage = async (assistantMsgId: number) => {
    if (isThinking) return;
    const idx = messages.findIndex((m) => m.id === assistantMsgId);
    if (idx === -1) return;
    let prevUser: ChatMessage | null = null;
    for (let i = idx - 1; i >= 0; i -= 1) {
      if (messages[i].role === "user") {
        prevUser = messages[i];
        break;
      }
    }
    if (!prevUser) return;
    setMessages((prev) => prev.filter((m) => m.id !== assistantMsgId));
    await handleSendChat(prevUser.text, prevUser.attachments ?? [], true);
  };

  const handleEditMessage = async (userMsgId: number, newText: string) => {
    const clean = newText.trim();
    if (isThinking || !clean) return;
    const idx = messages.findIndex((m) => m.id === userMsgId && m.role === "user");
    if (idx === -1) return;
    const origAtts = messages[idx].attachments ?? [];
    const userTurnIndex = messages.slice(0, idx).filter((m) => m.role === "user").length;
    const convId = activeConversationIdRef.current;
    if (convId !== null) {
      try {
        await invoke("truncate_conversation_from_user_turn", {
          conversationId: convId,
          userTurnIndex,
        });
      } catch {
        // Proceed with local edit even if DB row wasn't persisted yet
      }
    }
    setMessages((prev) => prev.slice(0, idx));
    await handleSendChat(clean, origAtts, false);
  };

  const handleRenameConversation = async (id: number, title: string) => {
    const trimmed = title.trim();
    if (!trimmed) return;
    try {
      const updatedTitle = await invoke<string>("rename_conversation", {
        conversationId: id,
        title: trimmed,
      });
      setConversations((prev) =>
        prev.map((c) => (c.id === id ? { ...c, title: updatedTitle } : c))
      );
      await refreshConversations();
    } catch {
      // Ignore rename error
    }
  };

  const handleStopStream = () => {
    void invoke("cancel_chat_stream");
    activeStreamIdRef.current = null;
    typewriterRef.current.streamId = null;
    thinkingRef.current = false;
    setIsThinking(false);
    setActiveStreamId(null);
    applyVisualTargets("expanded", false);
  };

  const handleNewChat = async () => {
    if (isThinking) return;
    if (browserState?.active && !browserState.watching) {
      void invoke("close_builtin_browser").catch(() => {});
    }
    const curLang: UiLang = settings.ui_language === "zh" ? "zh" : "en";
    try {
      const created = await invoke<ConversationSummary>("create_conversation");
      activeConversationIdRef.current = created.id;
      setActiveConversationId(created.id);
      setMessages([makeWelcomeMessage(curLang)]);
      await refreshConversations();
    } catch {
      setMessages([makeWelcomeMessage(curLang)]);
    }
  };

  const handleToggleAgentMode = () => {
    if (isThinking) return;
    const next = { ...settings, agent_mode: !settings.agent_mode };
    setSettings(next);
    void invoke<AppSettings>("save_settings", {
      settings: next,
      newApiKey: null,
    })
      .then(setSettings)
      .catch(() => {});
  };

  const handleChangePermissionMode = (mode: AgentPermissionMode) => {
    setSettings((prev) => ({ ...prev, agent_permission_mode: mode }));
    void invoke<string>("set_agent_permission_mode", { mode })
      .then((normalized) => {
        setSettings((prev) => ({ ...prev, agent_permission_mode: normalized }));
      })
      .catch(() => {});
  };

  const windowAnimTimerRef = useRef<number | null>(null);

  const handleEnterWindowMode = () => {
    if (isWindowMode || windowAnimPhase !== "idle") return;
    if (windowAnimTimerRef.current) window.clearTimeout(windowAnimTimerRef.current);
    // Step 1: Softly fade/lift the compact island out before resizing the OS window
    setWindowAnimPhase("to-window");
    windowAnimTimerRef.current = window.setTimeout(() => {
      void invoke<boolean>("set_window_mode", { enabled: true, maximized: false })
        .then((max) => {
          setIsWindowMaximized(Boolean(max));
          setIsWindowMode(true);
          setWindowAnimPhase("idle");
        })
        .catch(() => {
          setIsWindowMode(true);
          setWindowAnimPhase("idle");
        });
    }, 120);
  };

  const handleExitWindowMode = (collapseToPill = false) => {
    if (!isWindowMode) {
      if (collapseToPill) transitionTo(restStateOrMusic());
      return;
    }
    if (windowAnimTimerRef.current) window.clearTimeout(windowAnimTimerRef.current);
    // Step 1: Smoothly shrink/fade the large window upward before resizing back to Dynamic Island
    setWindowAnimPhase(collapseToPill ? "leaving-pill" : "leaving-island");
    windowAnimTimerRef.current = window.setTimeout(() => {
      setIsWindowMode(false);
      setIsWindowMaximized(false);
      if (collapseToPill) {
        setWindowAnimPhase("idle");
        // Resizes back to island canvas and plays Buddy's native spring shrink-to-capsule animation
        void invoke("set_window_mode", { enabled: false })
          .then(() => {
            applyVisualTargets("expanded");
            requestAnimationFrame(() => transitionTo(restStateOrMusic()));
          })
          .catch(() => transitionTo(restStateOrMusic()));
      } else {
        setWindowAnimPhase("island-in");
        void invoke("set_window_mode", { enabled: false })
          .then(() => {
            applyVisualTargets("expanded");
            windowAnimTimerRef.current = window.setTimeout(() => setWindowAnimPhase("idle"), 220);
          })
          .catch(() => setWindowAnimPhase("idle"));
      }
    }, 165);
  };

  const handleToggleWindowMaximize = () => {
    if (!isWindowMode) return;
    setWindowAnimPhase("max-pulse");
    void invoke<boolean>("toggle_window_maximize")
      .then((max) => {
        setIsWindowMaximized(Boolean(max));
        window.setTimeout(() => {
          setWindowAnimPhase((cur) => (cur === "max-pulse" ? "idle" : cur));
        }, 210);
      })
      .catch(() => setWindowAnimPhase("idle"));
  };

  useEffect(() => {
    if (buddyState !== "expanded" && isWindowMode) {
      setIsWindowMode(false);
      setIsWindowMaximized(false);
    }
  }, [buddyState, isWindowMode]);

  const handleSelectChatModel = async (profileId: string) => {
    if (isThinking) return;
    try {
      const updated = await invoke<AppSettings>("select_role_models", {
        chatProfileId: profileId,
        watchProfileId: settings.watch_profile_id ?? "",
      });
      setSettings(updated);
    } catch {
      // Ignore model switch failure
    }
  };

  const handleResolveApproval = (stepId: string, decision: ApprovalDecision) => {
    void invoke("resolve_agent_approval", { stepId, decision }).catch(() => {});
  };

  const handleSettingsSaved = (updated: AppSettings) => {
    setSettings(updated);
    const nextLang: UiLang = updated.ui_language === "zh" ? "zh" : "en";
    setMessages((prev) =>
      prev.length === 1 && prev[0].id === 1 ? [makeWelcomeMessage(nextLang)] : prev
    );
  };

  useEffect(() => {
    if (whisperTimerRef.current) {
      window.clearTimeout(whisperTimerRef.current);
      whisperTimerRef.current = null;
    }

    if (buddyState === "expanded" && floatingBubble) {
      if (bubbleExitTimerRef.current) {
        window.clearTimeout(bubbleExitTimerRef.current);
        bubbleExitTimerRef.current = null;
      }
      setBubbleVisible(false);
      floatingBubbleRef.current = null;
      setFloatingBubble(null);
      setWhisperHovered(false);
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
      return;
    }

    if (!floatingBubble) {
      floatingBubbleRef.current = null;
      setBubbleVisible(false);
      setWhisperHovered(false);
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
      return;
    }

    const visible =
      buddyState !== "expanded" &&
      buddyState !== "stealth" &&
      (floatingBubble.kind !== "share_whisper" || Boolean(shareState?.active));

    if (visible && whisperRef.current) {
      const el = whisperRef.current;
      const bounds: [number, number, number, number] = [
        el.offsetLeft,
        el.offsetTop,
        el.offsetWidth,
        el.offsetHeight,
      ];
      void invoke("set_whisper_bounds", { bounds }).catch(() => {});
      if (!whisperHovered) {
        whisperTimerRef.current = window.setTimeout(() => {
          dismissBubbleAnimated();
        }, floatingBubble.durationMs);
      }
    } else {
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
    }

    return () => {
      if (whisperTimerRef.current) {
        window.clearTimeout(whisperTimerRef.current);
      }
    };
  }, [floatingBubble, shareState?.active, buddyState, dockEdge, layoutDims, whisperHovered]);

  useEffect(() => {
    const curLang: UiLang = settings.ui_language === "zh" ? "zh" : "en";
    const unlisten = listen<ReminderDuePayload>("buddy://reminder-due", (event) => {
      const rem = event.payload;
      if (!rem) return;
      if (stateRef.current === "expanded") {
        setDueReminderBanner(rem);
      } else {
        presentBubble({
          kind: "reminder",
          reminderId: rem.id,
          title: rem.title,
          prefix: getStrings(curLang).chat.reminderPrefix,
          durationMs: 28_000,
        });
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [settings.ui_language]);

  const handleOpenBubble = async () => {
    if (!floatingBubble) return;
    const bubble = floatingBubble;
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
      bubbleExitTimerRef.current = null;
    }
    setBubbleVisible(false);
    floatingBubbleRef.current = null;
    setFloatingBubble(null);
    setWhisperHovered(false);
    if (bubble.kind === "reminder") {
      void invoke("snooze_reminder", { id: bubble.reminderId, minutes: 5 }).catch(() => {});
      return;
    }
    if (bubble.kind === "reply_notice") {
      switchTab("chat");
      transitionTo("expanded");
      return;
    }
    const w = bubble.whisper;
    let convId = activeConversationIdRef.current;
    if (convId === null) {
      try {
        convId = await invoke<number>("ensure_active_conversation");
        activeConversationIdRef.current = convId;
        setActiveConversationId(convId);
      } catch {
        convId = 1;
      }
    }
    try {
      await invoke("accept_share_whisper", { conversationId: convId, text: w.text });
    } catch {
      // ignore
    }
    setMessages((prev) => [...prev, { id: Date.now(), role: "assistant", text: w.text }]);
    transitionTo("expanded");
  };

  const handleSelectConversation = (convId: number) => {
    if (isThinking) return;
    void loadConversationMessages(convId);
  };

  const handleDeleteConversation = async (convId: number) => {
    try {
      await invoke("delete_conversation", { conversationId: convId });
      if (activeConversationIdRef.current === convId) {
        const nextId = await invoke<number>("ensure_active_conversation");
        await loadConversationMessages(nextId);
      }
      await refreshConversations();
    } catch {
      // ignore
    }
  };

  const handleCheckUpdate = async (): Promise<{ info: UpdateInfo | null; error?: string }> => {
    try {
      const info = await invoke<UpdateInfo | null>("check_app_update");
      if (info) setAvailableUpdate(info);
      return { info };
    } catch (err) {
      return { info: null, error: String(err) };
    }
  };

  const handleInstallUpdate = () => {
    if (updateProgress !== null) return;
    setUpdateProgress(0);
    void invoke("install_app_update").catch(() => {
      setUpdateProgress(null);
    });
  };

  const handleResetDock = () => {
    void invoke<DockStatePayload>("reset_dock_position")
      .then((res) => {
        const edge = (res.dock_edge || "top") as DockEdge;
        dockEdgeRef.current = edge;
        setDockEdge(edge);
        snapPreviewRef.current = null;
        setSnapPreview(null);
        applyVisualTargets(stateRef.current);
      })
      .catch(() => {});
  };

  const handleDragPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("button, input, textarea, select, a")) return;
    // When collapsed, expand/drag ONLY if click lands directly inside Buddy's visible capsule body.
    if (stateRef.current !== "expanded" && pillRectRef.current) {
      const pr = pillRectRef.current.getBoundingClientRect();
      if (
        e.clientX < pr.left ||
        e.clientX > pr.right ||
        e.clientY < pr.top ||
        e.clientY > pr.bottom
      ) {
        return;
      }
    }
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }
    dragMetaRef.current = {
      active: true,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      moved: false,
    };
  };

  const handleDragPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const meta = dragMetaRef.current;
    if (!meta || !meta.active || meta.pointerId !== e.pointerId) return;

    if (!meta.moved) {
      const dist = Math.hypot(e.clientX - meta.startX, e.clientY - meta.startY);
      if (dist <= 5) return;
      meta.moved = true;
      isDraggingRef.current = true;
      void invoke("begin_buddy_drag");
      return;
    }

    if (dragStepInFlightRef.current) return;
    dragStepInFlightRef.current = true;
    void invoke<DockStatePayload>("step_buddy_drag")
      .then((res) => {
        const edge = (res.dock_edge || "free") as DockEdge;
        if (edge !== dockEdgeRef.current) {
          dockEdgeRef.current = edge;
          setDockEdge(edge);
          applyVisualTargets(stateRef.current);
        }
        const nextSnap = (res.snap_preview ?? null) as DockEdge | null;
        if (nextSnap !== snapPreviewRef.current) {
          snapPreviewRef.current = nextSnap;
          setSnapPreview(nextSnap);
        }
      })
      .catch(() => {})
      .finally(() => {
        dragStepInFlightRef.current = false;
      });
  };

  const handleDragPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    const meta = dragMetaRef.current;
    if (!meta || !meta.active || meta.pointerId !== e.pointerId) return;
    dragMetaRef.current = null;
    try {
      if (e.currentTarget.hasPointerCapture(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    } catch {
      // ignore
    }

    if (meta.moved) {
      isDraggingRef.current = false;
      void invoke<DockStatePayload>("end_buddy_drag")
        .then((res) => {
          const edge = (res.dock_edge || "top") as DockEdge;
          dockEdgeRef.current = edge;
          setDockEdge(edge);
          snapPreviewRef.current = null;
          setSnapPreview(null);
          applyVisualTargets(stateRef.current);
        })
        .catch(() => {});
    } else if (stateRef.current !== "expanded") {
      transitionTo("expanded");
    }
  };

  const isExpanded = buddyState === "expanded";
  const sideDocked = isSideDock(dockEdge);
  const enabledPluginsMap: Record<string, boolean> = {};
  const installedPluginsMap: Record<string, boolean> = {};
  for (const p of pluginsData.plugins) {
    enabledPluginsMap[p.id] = p.enabled;
    installedPluginsMap[p.id] = p.installed;
  }

  const handleQuickOcr = (source: string) => {
    void invoke<OcrResult>("plugin_offline_ocr", { source })
      .then((res) => {
        const txt = res.text.trim() || "(no text detected)";
        setMessages((prev) => [
          ...prev,
          {
            id: Date.now(),
            role: "assistant",
            text: `**Offline OCR (${res.language}):**\n\`\`\`\n${txt}\n\`\`\``,
          },
        ]);
        switchTab("chat");
      })
      .catch((err) => {
        setMessages((prev) => [
          ...prev,
          { id: Date.now(), role: "assistant", text: String(err), isError: true },
        ]);
      });
  };

  const handleQuickDownloadUrl = (url: string, format: string) => {
    setChatInput("");
    setMessages((prev) => [
      ...prev,
      { id: Date.now(), role: "user", text: `${url} (${format})` },
    ]);
    void invoke<PluginToolOutput>("plugin_download_media", {
      url,
      format,
      outputDir: null,
    })
      .then((res) => {
        setMessages((prev) => [
          ...prev,
          {
            id: Date.now() + 1,
            role: "assistant",
            text: `**${res.summary}**\n\`${res.outputs[0] ?? ""}\``,
            outputPaths: res.outputs,
          },
        ]);
      })
      .catch((err) => {
        setMessages((prev) => [
          ...prev,
          { id: Date.now() + 1, role: "assistant", text: String(err), isError: true },
        ]);
      });
  };
  const dims = layoutDims;
  const activeDims: [number, number] = sideDocked
    ? [dims.h_w, dims.h_h]
    : [dims.v_w, dims.v_h];
  const cardPosStyle: React.CSSProperties =
    dockEdge === "left"
      ? {
          left: dims.pad,
          top: (dims.canvas_h - dims.h_h) / 2,
          width: dims.h_w,
          height: dims.h_h,
          flexDirection: "row",
        }
      : dockEdge === "right"
      ? {
          left: dims.canvas_w - dims.pad - dims.h_w,
          top: (dims.canvas_h - dims.h_h) / 2,
          width: dims.h_w,
          height: dims.h_h,
          flexDirection: "row-reverse",
        }
      : dockEdge === "bottom"
      ? {
          left: (dims.canvas_w - dims.v_w) / 2,
          top: dims.canvas_h - dims.pad - dims.v_h,
          width: dims.v_w,
          height: dims.v_h,
          flexDirection: "column-reverse",
        }
      : {
          left: (dims.canvas_w - dims.v_w) / 2,
          top: dims.pad,
          width: dims.v_w,
          height: dims.v_h,
          flexDirection: "column",
        };

  const lang: UiLang = settings.ui_language === "zh" ? "zh" : "en";
  langRef.current = lang;
  const activeChatModel = resolveActiveChatModel(settings);
  const thinkingCapability = getModelThinkingCapability(activeChatModel);
  const thinkingLevel: ThinkingLevel = clampThinkingLevel(
    normalizeThinkingLevel(settings.thinking_level),
    thinkingCapability
  );
  thinkingLevelRef.current = thinkingLevel;
  const ui = getStrings(lang);

  const workbenchContent = (
    <>
      {dueReminderBanner && (
        <div className="mb-2 rounded-xl bg-amber-500/15 border border-amber-400/35 px-2.5 py-1.5 flex items-center justify-between gap-2 shrink-0">
          <div className="min-w-0 flex items-center gap-1.5 text-[11px] text-amber-100">
            <Bell className="w-3.5 h-3.5 text-amber-300 shrink-0" />
            <span className="truncate font-medium">{dueReminderBanner.title}</span>
          </div>
          <div className="flex items-center gap-1 shrink-0">
            <button
              type="button"
              onClick={() => {
                const rid = dueReminderBanner.id;
                setDueReminderBanner(null);
                void invoke("snooze_reminder", { id: rid, minutes: 5 }).catch(() => {});
              }}
              className="px-2 py-0.5 rounded-lg bg-amber-400/90 hover:bg-amber-300 text-[10px] font-semibold text-slate-950 transition"
            >
              {ui.chat.reminderSnooze}
            </button>
            <button
              type="button"
              onClick={() => setDueReminderBanner(null)}
              className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/80 transition"
            >
              {ui.chat.reminderDone}
            </button>
          </div>
        </div>
      )}

      {availableUpdate && (
        <div className="mb-2 rounded-xl bg-white/[0.08] border border-white/15 px-2.5 py-1.5 flex items-center justify-between gap-2 shrink-0">
          <div className="min-w-0 text-[11px] text-white/90 truncate">
            <span className="font-medium">{ui.updateBanner}:</span>{" "}
            <span className="font-mono text-white">v{availableUpdate.version}</span>
          </div>
          <button
            type="button"
            disabled={updateProgress !== null}
            onClick={handleInstallUpdate}
            className="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-white hover:bg-white/90 disabled:opacity-60 text-[10.5px] font-medium text-black transition"
          >
            <Download className="w-3 h-3" />
            <span>
              {updateProgress !== null
                ? `${ui.updatingPct} ${updateProgress}%`
                : ui.updateNow}
            </span>
          </button>
        </div>
      )}

      {activeTab === "chat" && (
        <ChatTab
          lang={lang}
          isWindowMode={isWindowMode}
          agentMode={settings.agent_mode}
          permissionMode={
            settings.agent_permission_mode === "ask" ||
            settings.agent_permission_mode === "auto"
              ? settings.agent_permission_mode
              : "default"
          }
          thinkingLevel={thinkingLevel}
          activeStreamId={activeStreamId}
          messages={messages}
          conversations={conversations}
          activeConversationId={activeConversationId}
          chatInput={chatInput}
          onChangeInput={setChatInput}
          onFocusInput={(focused) => {
            focusedRef.current = focused;
          }}
          isThinking={isThinking}
          attachments={attachments}
          recentMemory={recentMemory}
          onDismissRecent={() => setRecentMemory(null)}
          onRemoveAttachment={(idx) =>
            setAttachments((prev) => prev.filter((_, i) => i !== idx))
          }
          onAddAttachment={(att) =>
            setAttachments((prev) => [...prev, att].slice(0, 6))
          }
          onAddAttachments={(atts) =>
            setAttachments((prev) => [...prev, ...atts].slice(0, 6))
          }
          onSend={(p, a) => void handleSendChat(p, a)}
          onRetryMessage={(id) => void handleRetryMessage(id)}
          onEditMessage={(id, text) => void handleEditMessage(id, text)}
          onStopStream={handleStopStream}
          onToggleAgentMode={handleToggleAgentMode}
          onChangePermissionMode={handleChangePermissionMode}
          onResolveApproval={handleResolveApproval}
          onNewChat={() => void handleNewChat()}
          onSelectConversation={handleSelectConversation}
          onDeleteConversation={(id) => void handleDeleteConversation(id)}
          onRenameConversation={(id, title) => void handleRenameConversation(id, title)}
          onOpenSettings={() => switchTab("settings")}
          enabledPlugins={enabledPluginsMap}
          installedPlugins={installedPluginsMap}
          skills={skills}
          onOpenPlugin={(pluginId, paths) => {
            if (paths && paths.length > 0) setDroppedPathsHint(paths);
            setFocusedPluginId(pluginId);
            switchTab("plugins");
          }}
          onQuickOcr={handleQuickOcr}
          onQuickDownloadUrl={handleQuickDownloadUrl}
          pluginProgress={pluginProgress}
          shareState={shareState}
          browserState={browserState}
          browserPreview={browserPreview}
          modelProfiles={settings.model_profiles ?? []}
          activeChatProfileId={settings.chat_profile_id ?? ""}
          activeModelName={settings.model_id}
          onSelectChatModel={(id) => void handleSelectChatModel(id)}
          defaultWatchProfileId={settings.watch_profile_id ?? ""}
        />
      )}

      {(activeTab === "memory" || activeTab === "plugins" || activeTab === "settings") && (
        <Suspense
          fallback={
            <div className="flex-1 flex items-center justify-center text-white/45">
              <Loader2 className="w-4 h-4 animate-spin" />
            </div>
          }
        >
          {activeTab === "memory" && (
            <MemoryTab
              lang={lang}
              isWindowMode={isWindowMode}
              refreshTrigger={memoryRefresh}
              enabledPlugins={enabledPluginsMap}
              onAskAi={(item) => {
                setAttachments((prev) => [...prev, memoryToAttachment(item)].slice(0, 6));
                switchTab("chat");
              }}
              onOcrExtracted={(ocrText) => {
                setMessages((prev) => [
                  ...prev,
                  {
                    id: Date.now(),
                    role: "assistant",
                    text: `**Offline OCR:**\n\`\`\`\n${ocrText}\n\`\`\``,
                  },
                ]);
                switchTab("chat");
              }}
            />
          )}

          {activeTab === "plugins" && (
            <PluginsTab
              lang={lang}
              isWindowMode={isWindowMode}
              pluginsData={pluginsData}
              pluginProgress={pluginProgress}
              lastPickedColor={lastPickedColor}
              timerState={timerState}
              droppedPathsHint={droppedPathsHint}
              focusedPluginId={focusedPluginId}
              skills={skills}
              onRefreshPlugins={setPluginsData}
              onRefreshSkills={setSkills}
              onCopyText={(text) => {
                void navigator.clipboard?.writeText(text).catch(() => {});
              }}
            />
          )}

          {activeTab === "settings" && (
            <SettingsTab
              isWindowMode={isWindowMode}
              settings={settings}
              refreshHz={refreshHz}
              expandedDims={activeDims}
              appVersion={appVersion}
              availableUpdate={availableUpdate}
              updateProgress={updateProgress}
              onSaved={handleSettingsSaved}
              onTriggerSnip={() => {
                handleExitWindowMode(true);
                window.setTimeout(() => void invoke("trigger_snip"), 180);
              }}
              onTestInhale={() => {
                handleExitWindowMode(true);
                window.setTimeout(triggerCapture, 220);
              }}
              onCheckUpdate={handleCheckUpdate}
              onInstallUpdate={handleInstallUpdate}
              onResetDock={handleResetDock}
            />
          )}
        </Suspense>
      )}
    </>
  );

  return (
    <div className="w-full h-full bg-transparent relative overflow-hidden pointer-events-none">
      {/* FULL DESKTOP WINDOW MODE */}
      {isWindowMode && (
        <div
          className={`fixed inset-0 z-40 pointer-events-auto flex flex-col bg-[#050507] text-white overflow-hidden transition-[opacity,transform,border-radius] duration-200 ease-out ${
            isWindowMaximized ? "rounded-none" : "rounded-2xl border border-white/[0.14] shadow-2xl"
          } ${
            windowAnimPhase === "leaving-pill"
              ? "opacity-0 scale-[0.91] -translate-y-7"
              : windowAnimPhase === "leaving-island"
              ? "opacity-0 scale-[0.94] -translate-y-4"
              : windowAnimPhase === "max-pulse"
              ? "buddy-window-pulse"
              : "buddy-window-enter"
          }`}
        >
          {/* Draggable Window Title Bar */}
          <div
            onMouseDown={(e) => {
              if ((e.target as HTMLElement).closest("button")) return;
              if (e.detail === 2) {
                handleToggleWindowMaximize();
              } else {
                void invoke("start_window_mode_drag").catch(() => {});
              }
            }}
            className="h-12 px-5 border-b border-white/[0.08] bg-black/80 flex items-center justify-between gap-4 shrink-0 select-none cursor-grab active:cursor-grabbing"
          >
            <div className="flex items-center gap-2.5 shrink-0 w-36">
              <div className="flex items-center gap-1.5">
                <span className={`w-[6px] h-[14px] rounded-full bg-white ${isThinking ? "animate-pulse" : ""}`} />
                <span className={`w-[6px] h-[14px] rounded-full bg-white ${isThinking ? "animate-pulse" : ""}`} />
              </div>
              <span className="text-[14px] font-semibold tracking-tight text-white">Buddy</span>
            </div>

            <div className="flex items-center gap-1 rounded-full bg-white/[0.045] border border-white/[0.09] p-1">
              {(
                [
                  { id: "chat", label: ui.tabs.chat, Icon: MessageSquare },
                  { id: "memory", label: ui.tabs.memory, Icon: Clock },
                  { id: "plugins", label: ui.tabs.plugins, Icon: Puzzle },
                  { id: "settings", label: ui.tabs.settings, Icon: Settings },
                ] as const
              ).map(({ id, label, Icon }) => {
                const active = activeTab === id;
                return (
                  <button
                    key={id}
                    type="button"
                    onClick={() => switchTab(id)}
                    className={`flex items-center gap-1.5 px-3.5 py-1 rounded-full text-[12.5px] font-medium transition ${
                      active
                        ? "bg-white text-black"
                        : "text-white/55 hover:text-white hover:bg-white/[0.06]"
                    }`}
                  >
                    <Icon className="w-3.5 h-3.5 shrink-0" />
                    <span>{label}</span>
                  </button>
                );
              })}
            </div>

            <div className="flex items-center justify-end gap-1.5 shrink-0 w-36">
              <button
                type="button"
                onClick={handleToggleWindowMaximize}
                title={isWindowMaximized ? ui.restoreWindowTitle : ui.maximizeWindowTitle}
                className="w-8 h-8 rounded-lg bg-white/[0.06] hover:bg-white/[0.15] text-white/75 hover:text-white flex items-center justify-center transition"
              >
                {isWindowMaximized ? (
                  <Minimize2 className="w-3.5 h-3.5" />
                ) : (
                  <Maximize2 className="w-3.5 h-3.5" />
                )}
              </button>
              <button
                type="button"
                onClick={() => handleExitWindowMode(false)}
                title={ui.exitWindowModeTitle}
                className="px-2.5 h-8 rounded-lg bg-white/[0.06] hover:bg-white/[0.15] text-[11.5px] text-white/80 hover:text-white inline-flex items-center gap-1.5 transition"
              >
                <PanelTopClose className="w-3.5 h-3.5" />
                <span>{lang === "zh" ? "灵动岛" : "Island"}</span>
              </button>
              <button
                type="button"
                onClick={() => handleExitWindowMode(true)}
                title={ui.collapseTitle}
                className="w-8 h-8 rounded-lg bg-white/[0.06] hover:bg-rose-500/30 text-white/75 hover:text-white flex items-center justify-center transition"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
          </div>

          {/* Spacious Full Window Body */}
          <div className={`flex-1 flex flex-col min-h-0 ${activeTab === "chat" ? "p-0" : "px-6 py-4"}`}>
            {workbenchContent}
          </div>
        </div>
      )}

      {/* DYNAMIC ISLAND CAPSULE & COMPACT PANEL */}
      <div
        ref={morphLayerRef}
        className={`relative pointer-events-none transition-[opacity,transform] duration-150 ease-out ${
          isWindowMode
            ? "hidden"
            : windowAnimPhase === "to-window"
            ? "opacity-0 scale-[0.95]"
            : windowAnimPhase === "island-in"
            ? "buddy-window-enter"
            : "opacity-100"
        }`}
        style={{
          width: dims.canvas_w,
          height: dims.canvas_h,
          transformOrigin: "center center",
          willChange: isExpanded ? undefined : "transform",
        }}
      >
        <svg
          width={dims.canvas_w}
          height={dims.canvas_h}
          viewBox={`0 0 ${dims.canvas_w} ${dims.canvas_h}`}
          className="absolute top-0 left-0 pointer-events-none overflow-visible"
        >
          <rect
            ref={pillShadowRef}
            fill="rgba(0,0,0,0.34)"
          />
          <rect
            ref={pillRectRef}
            fill="#050507"
            stroke={snapPreview ? "rgba(56,189,248,0.65)" : "rgba(255,255,255,0.14)"}
            strokeWidth={snapPreview ? "1.6" : "1"}
          />
        </svg>

        {/* Magnetic Edge Snap Preview Glow */}
        {snapPreview && (
          <div
            className={`pointer-events-none absolute z-30 rounded-full bg-sky-400/90 shadow-[0_0_14px_rgba(56,189,248,0.85)] transition-opacity ${
              snapPreview === "left"
                ? "left-1 top-1/2 -translate-y-1/2 w-[3px] h-16"
                : snapPreview === "right"
                ? "right-1 top-1/2 -translate-y-1/2 w-[3px] h-16"
                : snapPreview === "bottom"
                ? "bottom-1 left-1/2 -translate-x-1/2 h-[3px] w-20"
                : "top-1 left-1/2 -translate-x-1/2 h-[3px] w-20"
            }`}
          />
        )}

        <div
          ref={clipBoxRef}
          onPointerDown={handleDragPointerDown}
          onPointerMove={handleDragPointerMove}
          onPointerUp={handleDragPointerUp}
          onPointerCancel={handleDragPointerUp}
          className={`relative w-full h-full pointer-events-auto ${
            isExpanded ? "cursor-default" : "cursor-grab active:cursor-grabbing"
          }`}
          style={{ willChange: "clip-path" }}
        >
          <div
            ref={cardFrameRef}
            className="absolute flex"
            style={cardPosStyle}
          >
            {/* Capsule Timer Luminous Edge Progress Bar */}
            {timerState?.active && timerState.total_sec > 0 && (
              <div className="pointer-events-none absolute inset-x-3 bottom-0.5 h-[2.5px] rounded-full bg-white/10 overflow-hidden z-30">
                <div
                  className="h-full bg-gradient-to-r from-sky-400 to-emerald-300 transition-all duration-500"
                  style={{
                    width: `${Math.max(2, Math.round((timerState.remaining_sec / timerState.total_sec) * 100))}%`,
                  }}
                />
              </div>
            )}
            {/* HERO EYES STAGE */}
            <BuddyPillFace
              eyesStageRef={eyesStageRef}
              facePivotRef={facePivotRef}
              eyesPairRef={eyesPairRef}
              leftEyeShapeRef={leftEyeShapeRef}
              rightEyeShapeRef={rightEyeShapeRef}
              leftEyeGlowRef={leftEyeGlowRef}
              rightEyeGlowRef={rightEyeGlowRef}
              leftEyePillRef={leftEyePillRef}
              rightEyePillRef={rightEyePillRef}
              leftEyeGlintRef={leftEyeGlintRef}
              rightEyeGlintRef={rightEyeGlintRef}
              leftEyeArcRef={leftEyeArcRef}
              rightEyeArcRef={rightEyeArcRef}
              mouthSvgRef={mouthSvgRef}
              mouthShapeRef={mouthShapeRef}
              particlesRef={particlesRef}
              particleElsRef={particleElsRef}
              sleepZRef={sleepZRef}
              idleDotsRef={idleDotsRef}
              musicNoteRef={musicNoteRef}
              musicEqRef={musicEqRef}
              eqBarRefs={eqBarRefs}
              captureToastRef={captureToastRef}
              isExpanded={isExpanded}
              dockEdge={dockEdge}
              sideDocked={sideDocked}
              buddyState={buddyState}
              shareState={shareState}
              isMusicPlaying={musicRef.current.playing}
              captureToastLabel={ui.captureToast}
              shareResumeLabel={ui.chat.shareResume}
              sharePauseLabel={ui.chat.sharePause}
              shareStopLabel={ui.chat.shareStop}
              openWindowModeTitle={ui.openWindowModeTitle}
              collapseTitle={ui.collapseTitle}
              onOpenWindowMode={handleEnterWindowMode}
              onCollapse={() => transitionTo(restStateOrMusic())}
            />

            {/* PRE-MOUNTED EXPANDED WORKBENCH */}
            <div
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
              className={`flex-1 flex flex-col min-w-0 min-h-0 cursor-default transition-opacity duration-150 ${
                sideDocked
                  ? "px-3.5 pt-3 pb-2.5"
                  : dockEdge === "bottom"
                  ? "px-4 pt-3 pb-1.5"
                  : "px-4 pb-2.5"
              } ${
                isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"
              }`}
            >
              {!isWindowMode && workbenchContent}

              {/* ULTRA-MINIMALIST BOTTOM MICRO-DOCK */}
              <div className="mt-2 pt-2 border-t border-white/[0.08] flex items-center justify-center gap-2 shrink-0">
                {(
                  [
                    { id: "chat", label: ui.tabs.chat, Icon: MessageSquare },
                    { id: "memory", label: ui.tabs.memory, Icon: Clock },
                    { id: "plugins", label: ui.tabs.plugins, Icon: Puzzle },
                    { id: "settings", label: ui.tabs.settings, Icon: Settings },
                  ] as const
                ).map(({ id, label, Icon }) => {
                  const active = activeTab === id;
                  return (
                    <button
                      key={id}
                      onClick={() => switchTab(id)}
                      title={label}
                      className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] font-medium transition-all duration-200 ${
                        active
                          ? "bg-white/[0.11] border-white/[0.15] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.16)]"
                          : "border-transparent text-white/42 hover:text-white/80 hover:bg-white/[0.04]"
                      }`}
                    >
                      <Icon className="w-3.5 h-3.5 shrink-0" />
                      {active && <span>{label}</span>}
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Floating Whisper / Reply Mini Bubble (kept outside morphLayerRef so text renders crisp at integer pixels) */}
      {floatingBubble &&
        (floatingBubble.kind !== "share_whisper" || shareState?.active) &&
        !isExpanded &&
        buddyState !== "stealth" && (
          <FloatingBubbleOverlay
            bubble={floatingBubble}
            bubbleVisible={bubbleVisible}
            dockEdge={dockEdge}
            dims={dims}
            whisperRef={whisperRef}
            dismissLabel={
              floatingBubble.kind === "reminder"
                ? ui.chat.reminderDone
                : ui.chat.whisperDismiss
            }
            actionLabel={
              floatingBubble.kind === "reminder"
                ? ui.chat.reminderSnooze
                : floatingBubble.kind === "share_whisper"
                ? ui.chat.whisperReply
                : ui.chat.replyView
            }
            onHoverChange={setWhisperHovered}
            onDismiss={dismissBubbleAnimated}
            onOpen={() => void handleOpenBubble()}
          />
        )}
    </div>
  );
}
