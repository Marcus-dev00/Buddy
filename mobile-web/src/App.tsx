import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import {
  Bell,
  Camera,
  Check,
  CheckCircle2,
  ChevronDown,
  Clock,
  Cloud,
  Copy,
  Download,
  Headphones,
  Image as ImageIcon,
  Link2,
  Loader2,
  Maximize2,
  MessageSquare,
  Monitor,
  Paperclip,
  Plus,
  QrCode,
  RefreshCw,
  Send,
  ShieldAlert,
  Smartphone,
  Square,
  Trash2,
  Wifi,
  X,
  XCircle,
} from "lucide-react";
import {
  apiFetch,
  buildFileUrl,
  buildMemoryImageUrl,
  checkMobileAppUpdate,
  copyTextReliably,
  getLocalAppVersion,
  loadPairing,
  parsePairingInput,
  resolveAndConnect,
  savePairing,
  type MobileAgentStep,
  type MobileAttachment,
  type MobileConversation,
  type MobileMemory,
  type MobileMessage,
  type MobileReminder,
  type MobileUpdateInfo,
  type PairingConfig,
  type PingInfo,
} from "./api";
import { QrScannerModal } from "./components/QrScannerModal";

type MobileTab = "chat" | "vault" | "connect";
type VaultSub = "memories" | "reminders";
type MemorySubFilter = "all" | "images" | "clipboard";

const TAB_ORDER: MobileTab[] = ["chat", "vault", "connect"];

/** Frameless pure-white Buddy eyes matching the desktop icon aesthetic (with natural alive blink) */
function BuddyEyesLogo({ thinking }: { thinking: boolean }) {
  return (
    <div className="inline-flex items-center justify-center gap-1.5 px-1 shrink-0" aria-label="Buddy">
      <span
        className={`w-[7px] h-[16px] rounded-full bg-white transition-opacity ${
          thinking ? "animate-pulse" : "bm-eye-alive"
        }`}
      />
      <span
        className={`w-[7px] h-[16px] rounded-full bg-white transition-opacity ${
          thinking ? "animate-pulse" : "bm-eye-alive"
        }`}
      />
    </div>
  );
}

function extractSavedMediaPaths(steps?: MobileAgentStep[]): string[] {
  const out: string[] = [];
  if (!steps) return out;
  for (const s of steps) {
    if (s.tool_name === "download_media" && s.status === "done" && s.preview) {
      const m = /^Saved:\s*(.+)$/m.exec(s.preview);
      if (m?.[1]) {
        const p = m[1].trim();
        if (p && !out.includes(p)) out.push(p);
      }
    }
  }
  return out;
}

function formatShortTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  const d = new Date(ts);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(
    d.getMinutes()
  ).padStart(2, "0")}`;
}

export default function App() {
  const [pairing, setPairing] = useState<PairingConfig | null>(() => loadPairing());
  const [activeBase, setActiveBase] = useState<string>("");
  const [connMode, setConnMode] = useState<"offline" | "lan" | "remote">("offline");
  const [connecting, setConnecting] = useState(false);
  const [connError, setConnError] = useState<string | null>(null);
  const [pingInfo, setPingInfo] = useState<PingInfo | null>(null);
  const [tab, setTab] = useState<MobileTab>(() => (loadPairing() ? "chat" : "connect"));
  const [tabDir, setTabDir] = useState<"left" | "right">("right");

  const switchTab = useCallback((next: MobileTab) => {
    setTab((prev) => {
      if (prev === next) return prev;
      setTabDir(TAB_ORDER.indexOf(next) > TAB_ORDER.indexOf(prev) ? "right" : "left");
      return next;
    });
  }, []);

  // App Version & Auto-Update Check
  const localVersion = useMemo(() => getLocalAppVersion(), []);
  const [availableUpdate, setAvailableUpdate] = useState<MobileUpdateInfo | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateStatusMsg, setUpdateStatusMsg] = useState<string | null>(null);

  // Chat state
  const [conversations, setConversations] = useState<MobileConversation[]>([]);
  const [activeConvId, setActiveConvId] = useState<number | null>(null);
  const [showConvPicker, setShowConvPicker] = useState(false);
  const [messages, setMessages] = useState<MobileMessage[]>([]);
  const [chatInput, setChatInput] = useState("");
  const [pendingAtts, setPendingAtts] = useState<MobileAttachment[]>([]);
  const [isThinking, setIsThinking] = useState(false);
  const [activeStreamId, setActiveStreamId] = useState<string | null>(null);
  const [playingFile, setPlayingFile] = useState<string | null>(null);

  // On-Demand PC Desktop Peek Modal
  const [showDesktopPeek, setShowDesktopPeek] = useState(false);
  const [desktopFrame, setDesktopFrame] = useState<string>("");
  const [desktopTs, setDesktopTs] = useState<number>(0);
  const [grabbingDesktop, setGrabbingDesktop] = useState(false);

  // Vault (7-Day Memories & Reminders ONLY — no user-editable Facts tab)
  const [vaultSub, setVaultSub] = useState<VaultSub>("memories");
  const [memFilter, setMemFilter] = useState<MemorySubFilter>("all");
  const [memories, setMemories] = useState<MobileMemory[]>([]);
  const [reminders, setReminders] = useState<MobileReminder[]>([]);
  const [quickNote, setQuickNote] = useState("");
  const [remTitle, setRemTitle] = useState("");
  const [remTime, setRemTime] = useState("20m");
  const [dueToast, setDueToast] = useState<{ id: number; title: string } | null>(null);
  const [copiedMemId, setCopiedMemId] = useState<number | null>(null);
  const [expandedTextIds, setExpandedTextIds] = useState<Record<number, boolean>>({});
  const [lightboxMem, setLightboxMem] = useState<MobileMemory | null>(null);

  // Pairing & Scanner
  const [pairDraft, setPairDraft] = useState("");
  const [manualLan, setManualLan] = useState(pairing?.lan ?? "");
  const [manualRemote, setManualRemote] = useState(pairing?.remote ?? "");
  const [manualToken, setManualToken] = useState(pairing?.token ?? "");
  const [showQrScanner, setShowQrScanner] = useState(false);
  const [showManualConnect, setShowManualConnect] = useState(false);

  const chatEndRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const activeConvIdRef = useRef<number | null>(null);
  activeConvIdRef.current = activeConvId;
  const activeStreamIdRef = useRef<string | null>(null);
  activeStreamIdRef.current = activeStreamId;

  const token = pairing?.token ?? "";

  const nonEmptyConversations = useMemo(
    () => conversations.filter((c) => c.message_count > 0),
    [conversations]
  );

  // Silent update check on startup for native Android APK
  useEffect(() => {
    if (!localVersion.isNativeApk) return;
    void checkMobileAppUpdate(pairing?.dir).then((info) => {
      if (info && info.versionCode > localVersion.code) {
        setAvailableUpdate(info);
      }
    });
  }, [localVersion, pairing?.dir]);

  const handleManualCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateStatusMsg(null);
    const info = await checkMobileAppUpdate(pairing?.dir);
    setCheckingUpdate(false);
    if (!info) {
      setUpdateStatusMsg("检查更新失败，请确认网络畅通");
      return;
    }
    if (info.versionCode > localVersion.code) {
      setAvailableUpdate(info);
      setUpdateStatusMsg(`发现新版本 v${info.versionName}`);
    } else {
      setAvailableUpdate(null);
      setUpdateStatusMsg(`已是最新版本 (v${localVersion.name})`);
    }
  };

  const triggerApkUpdate = (url: string) => {
    if (window.BuddyAndroid?.openUpdateUrl) {
      window.BuddyAndroid.openUpdateUrl(url);
    } else {
      window.open(url, "_blank");
    }
  };

  const loadConversationMessages = useCallback(
    async (base: string, tok: string, cid: number) => {
      try {
        const res = await apiFetch<{
          messages: Array<{
            id: number;
            role: string;
            content: string;
            thought_text?: string | null;
            attachments_json?: string | null;
            agent_steps_json?: string | null;
          }>;
        }>(base, tok, `/api/conversations/${cid}/messages`);
        const mapped: MobileMessage[] = (res.messages ?? []).map((m) => ({
          id: m.id,
          role: m.role === "user" ? "user" : "assistant",
          content: m.content,
          thought: m.thought_text ?? undefined,
          attachments: m.attachments_json ? JSON.parse(m.attachments_json) : undefined,
          steps: m.agent_steps_json ? JSON.parse(m.agent_steps_json) : undefined,
        }));
        setMessages(mapped);
      } catch {
        // ignore
      }
    },
    []
  );

  const refreshConversations = useCallback(async (base: string, tok: string) => {
    try {
      const convRes = await apiFetch<{ conversations: MobileConversation[] }>(
        base,
        tok,
        "/api/conversations"
      );
      setConversations(convRes.conversations ?? []);
      return convRes.conversations ?? [];
    } catch {
      return [];
    }
  }, []);

  const refreshAllData = useCallback(
    async (base: string, tok: string) => {
      try {
        const [convRes, memRes, remRes] = await Promise.all([
          apiFetch<{ conversations: MobileConversation[] }>(base, tok, "/api/conversations"),
          apiFetch<{ memories: MobileMemory[] }>(base, tok, "/api/memories"),
          apiFetch<{ reminders: MobileReminder[] }>(base, tok, "/api/reminders"),
        ]);
        const convs = convRes.conversations ?? [];
        setConversations(convs);
        setMemories((memRes.memories ?? []).filter((x) => x.kind !== "fact"));
        setReminders(remRes.reminders ?? []);

        const nonEmpty = convs.filter((c) => c.message_count > 0);
        if (activeConvIdRef.current && convs.some((c) => c.id === activeConvIdRef.current)) {
          await loadConversationMessages(base, tok, activeConvIdRef.current);
        } else if (nonEmpty.length > 0 && activeConvIdRef.current === null && messages.length === 0) {
          setActiveConvId(nonEmpty[0].id);
          await loadConversationMessages(base, tok, nonEmpty[0].id);
        }
      } catch {
        // ignore
      }
    },
    [loadConversationMessages, messages.length]
  );

  const connectWithConfig = useCallback(
    async (cfg: PairingConfig) => {
      setConnecting(true);
      setConnError(null);
      const res = await resolveAndConnect(cfg);
      setConnecting(false);
      if (!res) {
        setConnMode("offline");
        setActiveBase("");
        setConnError(
          "无法连接到电脑 Buddy：请确认电脑已开机、设置 → 手机中的服务已开启（出门需开启 Cloudflare 隧道或 Tailscale）。"
        );
        return false;
      }
      savePairing(res.updatedConfig);
      setPairing(res.updatedConfig);
      setManualLan(res.updatedConfig.lan);
      setManualRemote(res.updatedConfig.remote);
      setManualToken(res.updatedConfig.token);
      setActiveBase(res.baseUrl);
      setConnMode(res.mode);
      setPingInfo(res.ping);
      setIsThinking(Boolean(res.ping.thinking));
      void refreshAllData(res.baseUrl, res.updatedConfig.token);
      return true;
    },
    [refreshAllData]
  );

  useEffect(() => {
    if (pairing) {
      void connectWithConfig(pairing);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Consume Android Share Intents
  const consumeAndroidShare = useCallback(
    async (rawJson?: string) => {
      const payloadStr = rawJson ?? window.BuddyAndroid?.consumePendingShare?.() ?? "";
      if (!payloadStr) return;
      try {
        const shared = JSON.parse(payloadStr) as {
          text?: string;
          filename?: string;
          mimeType?: string;
          dataBase64?: string;
        };
        if (shared.text) {
          setChatInput((prev) => (prev ? `${prev}\n${shared.text}` : shared.text!));
          setTab("chat");
        }
        if (shared.dataBase64 && activeBase && token) {
          const up = await apiFetch<{ attachment: MobileAttachment }>(
            activeBase,
            token,
            "/api/upload",
            {
              method: "POST",
              body: {
                filename: shared.filename || "shared_image.jpg",
                mime_type: shared.mimeType || "image/jpeg",
                data_base64: shared.dataBase64,
                save_to_memory: true,
              },
            }
          );
          if (up.attachment) {
            setPendingAtts((prev) => [...prev, up.attachment].slice(0, 6));
            setTab("chat");
          }
        }
      } catch {
        // ignore
      }
    },
    [activeBase, token]
  );

  useEffect(() => {
    window.__onBuddyAndroidShare = (payload: string) => {
      void consumeAndroidShare(payload);
    };
    window.__onBuddyQrScanned = (raw: string) => {
      const parsed = parsePairingInput(raw);
      if (parsed) {
        setShowQrScanner(false);
        void connectWithConfig(parsed).then((ok) => {
          if (ok) setTab("chat");
        });
      }
    };
    if (activeBase && token) {
      void consumeAndroidShare();
    }
  }, [activeBase, token, consumeAndroidShare, connectWithConfig]);

  // Live SSE Event Stream from Desktop Buddy
  useEffect(() => {
    if (!activeBase || !token) return;
    const es = new EventSource(
      `${activeBase}/api/events?token=${encodeURIComponent(token)}`
    );

    const onChunk = (ev: MessageEvent) => {
      try {
        const p = JSON.parse(ev.data) as { stream_id: string; delta: string };
        setMessages((prev) => {
          const idx = prev.findIndex((m) => String(m.id) === p.stream_id);
          if (idx === -1) {
            return [...prev, { id: Number(p.stream_id) || Date.now(), role: "assistant", content: p.delta }];
          }
          const copy = prev.slice();
          copy[idx] = { ...copy[idx], content: copy[idx].content + p.delta };
          return copy;
        });
      } catch {
        // ignore
      }
    };

    const onThought = (ev: MessageEvent) => {
      try {
        const p = JSON.parse(ev.data) as { stream_id: string; delta: string };
        setMessages((prev) =>
          prev.map((m) =>
            String(m.id) === p.stream_id
              ? { ...m, thought: (m.thought || "") + p.delta }
              : m
          )
        );
      } catch {
        // ignore
      }
    };

    const onStep = (ev: MessageEvent) => {
      try {
        const st = JSON.parse(ev.data) as MobileAgentStep;
        if (st.status === "waiting_approval") {
          window.BuddyAndroid?.showNotification?.(
            "Buddy 需要你的确认",
            st.summary,
            `approval_${st.step_id}`
          );
        }
        setMessages((prev) =>
          prev.map((m) => {
            if (String(m.id) !== st.stream_id) return m;
            const steps = (m.steps ?? []).slice();
            const i = steps.findIndex((x) => x.step_id === st.step_id);
            if (i >= 0) steps[i] = st;
            else steps.push(st);
            return { ...m, steps };
          })
        );
      } catch {
        // ignore
      }
    };

    const onDone = (ev: MessageEvent) => {
      try {
        const p = JSON.parse(ev.data) as { stream_id: string; full_text: string; full_thought: string };
        setIsThinking(false);
        setActiveStreamId(null);
        setMessages((prev) =>
          prev.map((m) =>
            String(m.id) === p.stream_id
              ? {
                  ...m,
                  content: p.full_text || m.content,
                  thought: p.full_thought || m.thought,
                }
              : m
          )
        );
        void refreshConversations(activeBase, token);
      } catch {
        setIsThinking(false);
      }
    };

    const onError = (ev: MessageEvent) => {
      try {
        const p = JSON.parse(ev.data) as { stream_id: string; error: string };
        setIsThinking(false);
        setActiveStreamId(null);
        setMessages((prev) =>
          prev.map((m) =>
            String(m.id) === p.stream_id ? { ...m, content: p.error, isError: true } : m
          )
        );
      } catch {
        setIsThinking(false);
      }
    };

    const onReminderDue = (ev: MessageEvent) => {
      try {
        const p = JSON.parse(ev.data) as { id: number; title: string };
        setDueToast(p);
        window.BuddyAndroid?.showNotification?.("Buddy 提醒", p.title, `rem_${p.id}`);
        void apiFetch<{ reminders: MobileReminder[] }>(activeBase, token, "/api/reminders")
          .then((r) => setReminders(r.reminders ?? []))
          .catch(() => {});
      } catch {
        // ignore
      }
    };

    const onRemindersChanged = () => {
      void apiFetch<{ reminders: MobileReminder[] }>(activeBase, token, "/api/reminders")
        .then((r) => setReminders(r.reminders ?? []))
        .catch(() => {});
    };

    const onMemCaptured = () => {
      void apiFetch<{ memories: MobileMemory[] }>(activeBase, token, "/api/memories")
        .then((r) => setMemories((r.memories ?? []).filter((x) => x.kind !== "fact")))
        .catch(() => {});
    };

    const onConvsChanged = (ev: MessageEvent) => {
      void refreshConversations(activeBase, token);
      try {
        const p = JSON.parse(ev.data) as { conversation_id?: number; deleted?: boolean };
        if (
          p.conversation_id &&
          p.conversation_id === activeConvIdRef.current &&
          !p.deleted &&
          !activeStreamIdRef.current
        ) {
          void loadConversationMessages(activeBase, token, p.conversation_id);
        }
      } catch {
        // ignore
      }
    };

    es.addEventListener("ai-chunk", onChunk);
    es.addEventListener("ai-thought", onThought);
    es.addEventListener("agent-step", onStep);
    es.addEventListener("ai-done", onDone);
    es.addEventListener("ai-error", onError);
    es.addEventListener("reminder-due", onReminderDue);
    es.addEventListener("reminders-changed", onRemindersChanged);
    es.addEventListener("memory-captured", onMemCaptured);
    es.addEventListener("conversations-changed", onConvsChanged);

    return () => {
      es.close();
    };
  }, [activeBase, token, refreshConversations, loadConversationMessages]);

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isThinking]);

  const handleSendChat = async (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!activeBase || !token) {
      setTab("connect");
      return;
    }
    const text = chatInput.trim();
    if (!text && pendingAtts.length === 0) return;

    const attsToSend = pendingAtts;
    setChatInput("");
    setPendingAtts([]);

    const userMsgId = Date.now();
    const streamId = String(userMsgId + 1);

    if (isThinking) {
      setMessages((prev) => [...prev, { id: userMsgId, role: "user", content: text }]);
      try {
        await apiFetch(activeBase, token, "/api/chat/send", {
          method: "POST",
          body: { conversation_id: activeConvId, prompt: text },
        });
      } catch {
        // ignore
      }
      return;
    }

    setMessages((prev) => [
      ...prev,
      {
        id: userMsgId,
        role: "user",
        content: text || "分析附件",
        attachments: attsToSend.length > 0 ? attsToSend : undefined,
      },
      { id: Number(streamId), role: "assistant", content: "" },
    ]);
    setIsThinking(true);
    setActiveStreamId(streamId);

    try {
      const res = await apiFetch<{ conversation_id: number; stream_id: string }>(
        activeBase,
        token,
        "/api/chat/send",
        {
          method: "POST",
          body: {
            conversation_id: activeConvId,
            stream_id: streamId,
            prompt: text,
            attachments: attsToSend,
          },
        }
      );
      if (res.conversation_id) {
        setActiveConvId(res.conversation_id);
        void refreshConversations(activeBase, token);
      }
    } catch (err) {
      setIsThinking(false);
      setActiveStreamId(null);
      setMessages((prev) =>
        prev.map((m) =>
          String(m.id) === streamId ? { ...m, content: String(err), isError: true } : m
        )
      );
    }
  };

  const handleStopStream = async () => {
    if (!activeBase || !token) return;
    setIsThinking(false);
    setActiveStreamId(null);
    await apiFetch(activeBase, token, "/api/chat/stop", { method: "POST" }).catch(() => {});
  };

  /** New Chat stays as local draft — NEVER creates empty rows in DB until first message is sent */
  const handleStartDraftChat = () => {
    setActiveConvId(null);
    setMessages([]);
    setShowConvPicker(false);
  };

  const handleResolveApproval = async (
    stepId: string,
    decision: "allow_once" | "allow_session" | "reject"
  ) => {
    if (!activeBase || !token) return;
    await apiFetch(activeBase, token, "/api/approvals/resolve", {
      method: "POST",
      body: { step_id: stepId, decision },
    }).catch(() => {});
  };

  const handlePickFiles = async (files: FileList | null) => {
    if (!files || !activeBase || !token) return;
    for (const file of Array.from(files).slice(0, 4)) {
      const dataUrl = await new Promise<string>((resolve) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result || ""));
        reader.readAsDataURL(file);
      });
      if (!dataUrl) continue;
      try {
        const res = await apiFetch<{ attachment: MobileAttachment }>(
          activeBase,
          token,
          "/api/upload",
          {
            method: "POST",
            body: {
              filename: file.name,
              mime_type: file.type || "application/octet-stream",
              data_base64: dataUrl,
              save_to_memory: file.type.startsWith("image/"),
            },
          }
        );
        if (res.attachment) {
          setPendingAtts((prev) => [...prev, res.attachment].slice(0, 6));
        }
      } catch {
        // ignore single failed file
      }
    }
  };

  const handleGrabDesktop = async () => {
    if (!activeBase || !token || grabbingDesktop) return;
    setShowDesktopPeek(true);
    setGrabbingDesktop(true);
    try {
      const r = await apiFetch<{ data_url: string; timestamp_ms: number }>(
        activeBase,
        token,
        "/api/desktop/screenshot"
      );
      setDesktopFrame(r.data_url);
      setDesktopTs(r.timestamp_ms);
    } catch {
      // ignore
    } finally {
      setGrabbingDesktop(false);
    }
  };

  const handleSaveFile = (url: string, filename: string) => {
    if (window.BuddyAndroid?.saveMediaFile) {
      window.BuddyAndroid.saveMediaFile(url, filename);
      return;
    }
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.target = "_blank";
    a.click();
  };

  const handlePairApply = async (raw: string) => {
    const parsed = parsePairingInput(raw);
    if (!parsed) {
      setConnError("配对码格式无效，请从电脑 Buddy「设置 → 手机」扫码或复制完整配对码。");
      return;
    }
    setPairDraft("");
    const ok = await connectWithConfig(parsed);
    if (ok) setTab("chat");
  };

  const handleCopyMemoryText = async (mem: MobileMemory) => {
    if (!mem.content_text) return;
    const ok = await copyTextReliably(mem.content_text);
    if (ok) {
      setCopiedMemId(mem.id);
      window.setTimeout(() => {
        setCopiedMemId((cur) => (cur === mem.id ? null : cur));
      }, 1500);
    }
  };

  const handleAskBuddyWithMemory = (mem: MobileMemory) => {
    const isImg = mem.kind === "screenshot" || mem.kind === "clipboard_image";
    if (isImg && mem.file_path) {
      setPendingAtts((prev) =>
        [
          ...prev,
          {
            kind: "image",
            label: `${mem.app_name || "Screenshot"}`,
            file_path: mem.file_path,
            text_content: mem.content_text,
          },
        ].slice(0, 6)
      );
    } else if (mem.content_text) {
      setPendingAtts((prev) =>
        [
          ...prev,
          {
            kind: "text",
            label: `${mem.app_name || "Clipboard"}`,
            text_content: mem.content_text,
          },
        ].slice(0, 6)
      );
    }
    setLightboxMem(null);
    setTab("chat");
  };

  const handleDeleteMemory = async (id: number) => {
    if (!activeBase || !token) return;
    await apiFetch(activeBase, token, `/api/memories/${id}`, { method: "DELETE" }).catch(() => {});
    setMemories((prev) => prev.filter((m) => m.id !== id));
    setLightboxMem((cur) => (cur?.id === id ? null : cur));
  };

  const activeConvTitle = useMemo(() => {
    if (activeConvId === null) return "新对话";
    const found = conversations.find((c) => c.id === activeConvId);
    return found && found.message_count > 0 ? found.title : "新对话";
  }, [conversations, activeConvId]);

  const imageMemories = useMemo(
    () => memories.filter((m) => m.kind === "screenshot" || m.kind === "clipboard_image"),
    [memories]
  );
  const textMemories = useMemo(
    () => memories.filter((m) => m.kind === "clipboard_text"),
    [memories]
  );

  return (
    <div className="h-full w-full flex flex-col bg-black text-white select-none overflow-hidden">
      {/* TOP HEADER — Pure Black Desktop-Matched */}
      <header className="safe-top px-4 pb-2.5 border-b border-white/[0.08] bg-black/95 backdrop-blur-xl flex items-center justify-between gap-2 shrink-0">
        <div className="flex items-center gap-2.5 min-w-0">
          <BuddyEyesLogo thinking={isThinking} />
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="text-[14px] font-semibold tracking-tight text-white">Buddy</span>
              <span
                className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                  connMode === "lan"
                    ? "bg-emerald-400"
                    : connMode === "remote"
                    ? "bg-white"
                    : "bg-white/25"
                }`}
              />
            </div>
            <div className="text-[10.5px] text-white/45 truncate">
              {connMode === "lan"
                ? `${pingInfo?.device_name || pairing?.name || "PC"} · 局域网`
                : connMode === "remote"
                ? `${pingInfo?.device_name || pairing?.name || "PC"} · 远程加密`
                : "未连接电脑"}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {tab === "chat" && (
            <>
              <button
                type="button"
                disabled={connMode === "offline"}
                onClick={() => void handleGrabDesktop()}
                title="抓拍电脑屏幕"
                className="w-8 h-8 rounded-full bg-white/[0.06] hover:bg-white/[0.14] border border-white/[0.1] text-white/80 disabled:opacity-35 flex items-center justify-center transition"
              >
                <Monitor className="w-4 h-4" />
              </button>
              <button
                type="button"
                onClick={handleStartDraftChat}
                title="新建对话"
                className="w-8 h-8 rounded-full bg-white/[0.06] hover:bg-white/[0.14] border border-white/[0.1] text-white/85 flex items-center justify-center transition"
              >
                <Plus className="w-4 h-4" />
              </button>
            </>
          )}
          {tab !== "chat" && (
            <button
              type="button"
              onClick={() => activeBase && token && void refreshAllData(activeBase, token)}
              title="刷新"
              className="w-8 h-8 rounded-full bg-white/[0.06] hover:bg-white/[0.14] border border-white/[0.1] text-white/75 flex items-center justify-center transition"
            >
              <RefreshCw className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
      </header>

      {/* UPDATE AVAILABLE BANNER (Android APK) */}
      {availableUpdate && (
        <div className="mx-3.5 mt-2 rounded-2xl bg-white/[0.07] border border-white/[0.16] px-3 py-2 flex items-center justify-between gap-2 shrink-0">
          <div className="min-w-0">
            <div className="text-[11.5px] font-medium text-white truncate">
              发现新版本 v{availableUpdate.versionName}
            </div>
            {availableUpdate.notes && (
              <div className="text-[10px] text-white/50 truncate">{availableUpdate.notes}</div>
            )}
          </div>
          <button
            type="button"
            onClick={() => triggerApkUpdate(availableUpdate.apkUrl)}
            className="px-2.5 py-1 rounded-full bg-white text-black text-[11px] font-semibold shrink-0"
          >
            更新
          </button>
        </div>
      )}

      {/* DUE REMINDER BANNER */}
      {dueToast && (
        <div className="mx-3.5 mt-2 rounded-2xl bg-amber-500/15 border border-amber-400/35 px-3 py-2 flex items-center justify-between gap-2 shrink-0">
          <div className="flex items-center gap-2 min-w-0 text-xs text-amber-100">
            <Bell className="w-4 h-4 text-amber-300 shrink-0" />
            <span className="truncate font-medium">{dueToast.title}</span>
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            <button
              type="button"
              onClick={() => {
                const rid = dueToast.id;
                setDueToast(null);
                if (activeBase && token) {
                  void apiFetch(activeBase, token, `/api/reminders/${rid}/snooze`, {
                    method: "POST",
                    body: { minutes: 5 },
                  });
                }
              }}
              className="px-2 py-0.5 rounded-full bg-amber-400 text-black text-[11px] font-semibold"
            >
              +5分
            </button>
            <button
              type="button"
              onClick={() => setDueToast(null)}
              className="px-2 py-0.5 rounded-full bg-white/10 text-[11px] text-white/80"
            >
              知道了
            </button>
          </div>
        </div>
      )}

      {/* MAIN CONTENT */}
      <main className="flex-1 min-h-0 flex flex-col overflow-hidden">
        {/* TAB 1: CHAT */}
        {tab === "chat" && (
          <div
            key="tab-chat"
            className={`flex-1 flex flex-col min-h-0 ${
              tabDir === "right" ? "bm-tab-right" : "bm-tab-left"
            }`}
          >
            {/* Compact History Bar */}
            <div className="px-3.5 py-2 border-b border-white/[0.06] flex items-center justify-between gap-2 shrink-0">
              <button
                type="button"
                onClick={() => setShowConvPicker((v) => !v)}
                className="flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-white/[0.05] hover:bg-white/[0.1] border border-white/[0.1] text-[11.5px] text-white/85 min-w-0 max-w-[75%] transition"
              >
                <Clock className="w-3 h-3 text-white/50 shrink-0" />
                <span className="truncate">{activeConvTitle}</span>
                <ChevronDown className="w-3 h-3 text-white/45 shrink-0" />
              </button>
              <span className="text-[10.5px] text-white/35 font-mono truncate">
                {pingInfo?.model_id || ""}
              </span>
            </div>

            {/* History Picker Sheet (No empty conversations; 📱 / 💻 icons) */}
            {showConvPicker && (
              <div className="bm-sheet-down px-3.5 py-2.5 bg-[#08080a] border-b border-white/[0.09] max-h-60 overflow-y-auto space-y-1.5 shrink-0">
                <div className="flex items-center justify-between px-1 pb-0.5">
                  <span className="text-[10.5px] text-white/45">
                    历史对话 ({nonEmptyConversations.length})
                  </span>
                  <button
                    type="button"
                    onClick={() => setShowConvPicker(false)}
                    className="text-[10.5px] text-white/55 hover:text-white"
                  >
                    收起
                  </button>
                </div>
                {nonEmptyConversations.length === 0 ? (
                  <div className="text-center py-4 text-xs text-white/40">暂无历史对话</div>
                ) : (
                  nonEmptyConversations.map((c) => {
                    const isCur = c.id === activeConvId;
                    const isMobile = c.source === "mobile";
                    return (
                      <div
                        key={c.id}
                        className={`flex items-center justify-between gap-2 px-2.5 py-2 rounded-xl border transition ${
                          isCur
                            ? "bg-white/[0.11] border-white/25 text-white"
                            : "bg-white/[0.03] border-white/[0.07] text-white/75"
                        }`}
                      >
                        <button
                          type="button"
                          onClick={() => {
                            setActiveConvId(c.id);
                            setShowConvPicker(false);
                            if (activeBase && token) {
                              void loadConversationMessages(activeBase, token, c.id);
                            }
                          }}
                          className="flex-1 min-w-0 flex items-center gap-2.5 text-left"
                        >
                          <span
                            className={`w-6 h-6 rounded-lg flex items-center justify-center shrink-0 ${
                              isMobile
                                ? "bg-emerald-500/15 text-emerald-300"
                                : "bg-white/[0.07] text-white/60"
                            }`}
                          >
                            {isMobile ? (
                              <Smartphone className="w-3.5 h-3.5" />
                            ) : (
                              <Monitor className="w-3.5 h-3.5" />
                            )}
                          </span>
                          <div className="min-w-0 flex-1">
                            <div className="text-xs font-medium truncate">{c.title}</div>
                            <div className="text-[10px] text-white/40">
                              {c.message_count} 条 · {formatShortTime(c.updated_at)}
                            </div>
                          </div>
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            if (!activeBase || !token) return;
                            void apiFetch(activeBase, token, `/api/conversations/${c.id}`, {
                              method: "DELETE",
                            }).then(() => {
                              setConversations((prev) => prev.filter((x) => x.id !== c.id));
                              if (activeConvId === c.id) {
                                setActiveConvId(null);
                                setMessages([]);
                              }
                            });
                          }}
                          className="p-1.5 text-white/35 hover:text-rose-300 shrink-0"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    );
                  })
                )}
              </div>
            )}

            {/* Message Feed */}
            <div className="flex-1 overflow-y-auto px-3.5 py-3 space-y-3 select-text">
              {connMode === "offline" && (
                <div className="rounded-2xl bg-white/[0.04] border border-white/[0.1] p-4 text-center space-y-2.5">
                  <div className="text-xs text-white/75">尚未连接到你的电脑 Buddy</div>
                  <button
                    type="button"
                    onClick={() => setTab("connect")}
                    className="px-4 py-1.5 rounded-full bg-white text-black text-xs font-semibold"
                  >
                    前往配对连接
                  </button>
                </div>
              )}

              {messages.length === 0 && connMode !== "offline" && (
                <div className="py-16 text-center space-y-3">
                  <div className="flex justify-center">
                    <BuddyEyesLogo thinking={false} />
                  </div>
                  <div className="text-xs text-white/45">
                    发消息、传照片或让电脑 Buddy 帮你干活
                  </div>
                </div>
              )}

              {messages.map((m) => {
                const mediaPaths = extractSavedMediaPaths(m.steps);
                return (
                  <div
                    key={m.id}
                    className={`bm-fade-up flex flex-col ${
                      m.role === "user" ? "items-end" : "items-start"
                    }`}
                  >
                    <div
                      className={`max-w-[88%] rounded-2xl px-3.5 py-2.5 text-[13.5px] leading-relaxed ${
                        m.role === "user"
                          ? "bg-white text-black rounded-br-sm"
                          : m.isError
                          ? "bg-rose-500/15 border border-rose-400/30 text-rose-100 rounded-bl-sm"
                          : "bg-[#111114] border border-white/[0.09] text-white/92 rounded-bl-sm"
                      }`}
                    >
                      {m.attachments && m.attachments.length > 0 && (
                        <div className="flex flex-wrap gap-1.5 mb-2">
                          {m.attachments.map((att, idx) => (
                            <span
                              key={idx}
                              className={`px-2 py-0.5 rounded-full text-[10px] font-mono max-w-[200px] truncate ${
                                m.role === "user"
                                  ? "bg-black/10 text-black/80"
                                  : "bg-white/[0.08] border border-white/10 text-white/80"
                              }`}
                            >
                              {att.label}
                            </span>
                          ))}
                        </div>
                      )}

                      {m.thought && (
                        <details className="mb-2 rounded-xl bg-black/50 border border-white/[0.08] px-2.5 py-1.5 text-[11px] text-white/55">
                          <summary className="cursor-pointer font-medium text-white/65">
                            思考过程
                          </summary>
                          <div className="mt-1 whitespace-pre-wrap">{m.thought}</div>
                        </details>
                      )}

                      {m.steps && m.steps.length > 0 && (
                        <div className="mb-2 space-y-1.5">
                          {m.steps.map((st) => (
                            <div
                              key={st.step_id}
                              className={`rounded-xl border px-2.5 py-2 text-[11px] ${
                                st.status === "waiting_approval"
                                  ? "bg-amber-500/12 border-amber-400/35 text-amber-100"
                                  : "bg-black/50 border-white/[0.08] text-white/75"
                              }`}
                            >
                              <div className="flex items-center gap-1.5 font-medium">
                                {st.status === "running" && (
                                  <Loader2 className="w-3 h-3 animate-spin text-white/75 shrink-0" />
                                )}
                                {st.status === "done" && (
                                  <CheckCircle2 className="w-3 h-3 text-emerald-400 shrink-0" />
                                )}
                                {(st.status === "error" || st.status === "rejected") && (
                                  <XCircle className="w-3 h-3 text-rose-400 shrink-0" />
                                )}
                                {st.status === "waiting_approval" && (
                                  <ShieldAlert className="w-3.5 h-3.5 text-amber-300 shrink-0" />
                                )}
                                <span className="truncate">{st.summary}</span>
                              </div>
                              {st.detail && (
                                <div className="mt-1 font-mono text-[10px] text-white/45 break-all">
                                  {st.detail}
                                </div>
                              )}
                              {st.status === "waiting_approval" && (
                                <div className="mt-2 flex items-center gap-1.5">
                                  <button
                                    type="button"
                                    onClick={() => void handleResolveApproval(st.step_id, "allow_once")}
                                    className="px-2.5 py-1 rounded-full bg-white text-black font-semibold text-[11px]"
                                  >
                                    允许一次
                                  </button>
                                  {!st.is_destructive && (
                                    <button
                                      type="button"
                                      onClick={() =>
                                        void handleResolveApproval(st.step_id, "allow_session")
                                      }
                                      className="px-2.5 py-1 rounded-full bg-white/12 border border-white/15 text-white text-[11px]"
                                    >
                                      本次都允许
                                    </button>
                                  )}
                                  <button
                                    type="button"
                                    onClick={() => void handleResolveApproval(st.step_id, "reject")}
                                    className="px-2.5 py-1 rounded-full bg-rose-500/20 border border-rose-400/35 text-rose-200 text-[11px]"
                                  >
                                    拒绝
                                  </button>
                                </div>
                              )}
                            </div>
                          ))}
                        </div>
                      )}

                      {m.content ? (
                        <div className="prose prose-invert prose-sm max-w-none break-words [&>p]:my-1 [&>pre]:bg-black/75 [&>pre]:p-2.5 [&>pre]:rounded-xl [&>pre]:overflow-x-auto">
                          <Markdown>{m.content}</Markdown>
                        </div>
                      ) : isThinking && String(m.id) === activeStreamId ? (
                        <span className="inline-flex items-center gap-1.5 text-xs text-white/50">
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                          <span>思考中...</span>
                        </span>
                      ) : null}

                      {/* Downloaded Media Player & Save Card */}
                      {mediaPaths.length > 0 && activeBase && token && (
                        <div className="mt-2.5 space-y-2 border-t border-white/10 pt-2">
                          {mediaPaths.map((mp) => {
                            const fname = mp.split(/[\\/]/).pop() || "media";
                            const streamUrl = buildFileUrl(activeBase, token, mp, false);
                            const dlUrl = buildFileUrl(activeBase, token, mp, true);
                            const isVideo = /\.(mp4|webm|mkv|mov)$/i.test(fname);
                            const isOpen = playingFile === mp;
                            return (
                              <div
                                key={mp}
                                className="rounded-xl bg-black/60 border border-white/[0.1] p-2 space-y-1.5"
                              >
                                <div className="flex items-center justify-between gap-2">
                                  <span className="text-[11px] font-mono text-white/85 truncate">
                                    {fname}
                                  </span>
                                  <div className="flex items-center gap-1 shrink-0">
                                    <button
                                      type="button"
                                      onClick={() => setPlayingFile(isOpen ? null : mp)}
                                      className="px-2 py-0.5 rounded-full bg-white text-black text-[10px] font-semibold inline-flex items-center gap-1"
                                    >
                                      <Headphones className="w-3 h-3" />
                                      <span>{isOpen ? "收起" : "播放"}</span>
                                    </button>
                                    <button
                                      type="button"
                                      onClick={() => handleSaveFile(dlUrl, fname)}
                                      className="px-2 py-0.5 rounded-full bg-white/10 text-white text-[10px] inline-flex items-center gap-1"
                                    >
                                      <Download className="w-3 h-3" />
                                      <span>保存</span>
                                    </button>
                                  </div>
                                </div>
                                {isOpen &&
                                  (isVideo ? (
                                    <video
                                      src={streamUrl}
                                      controls
                                      autoPlay
                                      className="w-full rounded-lg max-h-56 bg-black"
                                    />
                                  ) : (
                                    <audio src={streamUrl} controls autoPlay className="w-full h-9" />
                                  ))}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
              <div ref={chatEndRef} />
            </div>

            {/* Pending Attachments Strip */}
            {pendingAtts.length > 0 && (
              <div className="px-3.5 pt-1.5 flex items-center gap-1.5 overflow-x-auto shrink-0">
                {pendingAtts.map((a, idx) => (
                  <div
                    key={idx}
                    className="shrink-0 inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-white/[0.08] border border-white/[0.15] text-[11px] text-white/90 max-w-[170px]"
                  >
                    <span className="truncate">{a.label}</span>
                    <button
                      type="button"
                      onClick={() => setPendingAtts((prev) => prev.filter((_, i) => i !== idx))}
                      className="text-white/55 hover:text-white"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Chat Input Bar */}
            <form
              onSubmit={(e) => void handleSendChat(e)}
              className="px-3.5 py-2.5 border-t border-white/[0.08] bg-black flex items-end gap-2 shrink-0"
            >
              <input
                ref={fileInputRef}
                type="file"
                multiple
                accept="image/*,audio/*,video/*,.pdf,.txt,.md,.json,.csv"
                className="hidden"
                onChange={(e) => {
                  void handlePickFiles(e.target.files);
                  e.target.value = "";
                }}
              />
              <button
                type="button"
                onClick={() => fileInputRef.current?.click()}
                className="w-9 h-9 rounded-full bg-white/[0.07] hover:bg-white/[0.14] border border-white/[0.1] text-white/75 flex items-center justify-center shrink-0 transition"
                title="上传图片或文件"
              >
                <Paperclip className="w-4 h-4" />
              </button>
              <textarea
                rows={1}
                value={chatInput}
                onChange={(e) => setChatInput(e.target.value)}
                placeholder={isThinking ? "插话补充给 Buddy..." : "和电脑 Buddy 聊天..."}
                className="flex-1 rounded-2xl bg-[#111114] border border-white/[0.12] focus:border-white/35 outline-none px-3.5 py-2 text-[13.5px] text-white placeholder:text-white/35 resize-none max-h-28 overflow-y-auto"
              />
              {isThinking ? (
                <button
                  type="button"
                  onClick={() => void handleStopStream()}
                  className="w-9 h-9 rounded-full bg-rose-500/85 text-white flex items-center justify-center shrink-0"
                  title="停止"
                >
                  <Square className="w-3.5 h-3.5 fill-current" />
                </button>
              ) : (
                <button
                  type="submit"
                  disabled={!chatInput.trim() && pendingAtts.length === 0}
                  className="w-9 h-9 rounded-full bg-white text-black disabled:opacity-30 flex items-center justify-center shrink-0 transition"
                  title="发送"
                >
                  <Send className="w-4 h-4" />
                </button>
              )}
            </form>
          </div>
        )}

        {/* TAB 2: VAULT (7-DAY MEMORIES & REMINDERS ONLY) */}
        {tab === "vault" && (
          <div
            key="tab-vault"
            className={`flex-1 flex flex-col min-h-0 px-3.5 py-3 space-y-2.5 max-w-2xl mx-auto w-full ${
              tabDir === "right" ? "bm-tab-right" : "bm-tab-left"
            }`}
          >
            {/* Top 2-Way Segment: 7-Day Memory | Reminders */}
            <div className="grid grid-cols-2 gap-1 p-1 rounded-full bg-white/[0.05] border border-white/[0.1] shrink-0">
              {(
                [
                  { id: "memories", label: `7天记忆 (${memories.length})` },
                  {
                    id: "reminders",
                    label: `提醒 (${reminders.filter((r) => r.enabled).length})`,
                  },
                ] as const
              ).map((t) => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => setVaultSub(t.id)}
                  className={`py-1.5 rounded-full text-xs font-medium transition ${
                    vaultSub === t.id
                      ? "bg-white text-black font-semibold"
                      : "text-white/55 hover:text-white"
                  }`}
                >
                  {t.label}
                </button>
              ))}
            </div>

            {vaultSub === "memories" && (
              <>
                {/* Sub-Filter: All | Screenshots | Clipboard */}
                <div className="flex items-center gap-1.5 shrink-0">
                  {(
                    [
                      { id: "all", label: `全部 (${memories.length})` },
                      { id: "images", label: `截图 (${imageMemories.length})` },
                      { id: "clipboard", label: `剪贴板 (${textMemories.length})` },
                    ] as const
                  ).map((f) => (
                    <button
                      key={f.id}
                      type="button"
                      onClick={() => setMemFilter(f.id)}
                      className={`px-3 py-1 rounded-full border text-[11px] font-medium transition ${
                        memFilter === f.id
                          ? "bg-white/[0.14] border-white/25 text-white"
                          : "bg-white/[0.03] border-white/[0.08] text-white/50"
                      }`}
                    >
                      {f.label}
                    </button>
                  ))}
                </div>

                {/* Quick Feed Note into PC 7-Day Memory */}
                <form
                  onSubmit={async (e) => {
                    e.preventDefault();
                    if (!quickNote.trim() || !activeBase || !token) return;
                    const res = await apiFetch<{ memory: MobileMemory }>(
                      activeBase,
                      token,
                      "/api/memories/note",
                      { method: "POST", body: { text: quickNote.trim() } }
                    );
                    setMemories((prev) => [res.memory, ...prev]);
                    setQuickNote("");
                  }}
                  className="flex items-center gap-1.5 shrink-0"
                >
                  <input
                    type="text"
                    value={quickNote}
                    onChange={(e) => setQuickNote(e.target.value)}
                    placeholder="把文字或链接丢进电脑 7 天记忆..."
                    className="flex-1 rounded-xl bg-[#111114] border border-white/[0.11] focus:border-white/30 px-3 py-2 text-xs text-white placeholder:text-white/35 outline-none"
                  />
                  <button
                    type="submit"
                    disabled={!quickNote.trim()}
                    className="px-3.5 py-2 rounded-xl bg-white text-black disabled:opacity-35 text-xs font-semibold shrink-0"
                  >
                    存入
                  </button>
                </form>

                {/* Separated Screenshots Grid + Clipboard Text List */}
                <div key={`mem-${memFilter}`} className="bm-fade-up flex-1 overflow-y-auto space-y-4 pb-2">
                  {/* SECTION A: SCREENSHOTS */}
                  {(memFilter === "all" || memFilter === "images") && imageMemories.length > 0 && (
                    <div className="space-y-2">
                      {memFilter === "all" && (
                        <div className="flex items-center gap-1.5 px-0.5 text-[10.5px] font-medium uppercase tracking-wider text-white/45">
                          <ImageIcon className="w-3 h-3" />
                          <span>截图 · {imageMemories.length}</span>
                        </div>
                      )}
                      <div className="grid grid-cols-2 gap-2">
                        {imageMemories.map((mem) => (
                          <div
                            key={mem.id}
                            className="rounded-2xl bg-[#0e0e11] border border-white/[0.09] overflow-hidden flex flex-col"
                          >
                            <button
                              type="button"
                              onClick={() => setLightboxMem(mem)}
                              className="relative w-full h-28 bg-black overflow-hidden block"
                            >
                              {activeBase && token && (
                                <img
                                  src={buildMemoryImageUrl(activeBase, token, mem.id, false)}
                                  alt={mem.app_name || "截图"}
                                  loading="lazy"
                                  className="w-full h-full object-cover"
                                />
                              )}
                              <span className="absolute right-1.5 bottom-1.5 w-6 h-6 rounded-full bg-black/65 border border-white/15 text-white/85 flex items-center justify-center">
                                <Maximize2 className="w-3 h-3" />
                              </span>
                            </button>
                            <div className="px-2.5 py-2 flex items-center justify-between gap-1 text-[10px] text-white/50">
                              <span className="truncate">{mem.app_name || "截图"}</span>
                              <span className="shrink-0">{formatShortTime(mem.updated_at)}</span>
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* SECTION B: CLIPBOARD TEXT */}
                  {(memFilter === "all" || memFilter === "clipboard") && textMemories.length > 0 && (
                    <div className="space-y-2">
                      {memFilter === "all" && (
                        <div className="flex items-center gap-1.5 px-0.5 text-[10.5px] font-medium uppercase tracking-wider text-white/45">
                          <Copy className="w-3 h-3" />
                          <span>剪贴板文字 · {textMemories.length}</span>
                        </div>
                      )}
                      <div className="space-y-2">
                        {textMemories.map((mem) => {
                          const isExpanded = Boolean(expandedTextIds[mem.id]);
                          const isCopied = copiedMemId === mem.id;
                          return (
                            <div
                              key={mem.id}
                              className="rounded-2xl bg-[#0e0e11] border border-white/[0.09] p-3 space-y-2"
                            >
                              <div className="flex items-center justify-between text-[10px] text-white/45">
                                <span className="truncate">{mem.app_name || "剪贴板"}</span>
                                <span className="shrink-0">{formatShortTime(mem.updated_at)}</span>
                              </div>
                              {mem.content_text && (
                                <p
                                  onClick={() =>
                                    setExpandedTextIds((prev) => ({
                                      ...prev,
                                      [mem.id]: !prev[mem.id],
                                    }))
                                  }
                                  className={`text-[12.5px] text-white/90 whitespace-pre-wrap break-words select-text leading-relaxed ${
                                    isExpanded ? "" : "line-clamp-4"
                                  }`}
                                >
                                  {mem.content_text}
                                </p>
                              )}
                              <div className="flex items-center justify-between pt-1 border-t border-white/[0.06]">
                                <div className="flex items-center gap-1.5">
                                  <button
                                    type="button"
                                    onClick={() => void handleCopyMemoryText(mem)}
                                    className={`px-2.5 py-1 rounded-full text-[11px] font-medium inline-flex items-center gap-1 transition ${
                                      isCopied
                                        ? "bg-emerald-500/20 border border-emerald-400/35 text-emerald-200"
                                        : "bg-white/[0.08] hover:bg-white/[0.16] text-white/90"
                                    }`}
                                  >
                                    {isCopied ? (
                                      <>
                                        <Check className="w-3 h-3" />
                                        <span>已复制</span>
                                      </>
                                    ) : (
                                      <>
                                        <Copy className="w-3 h-3" />
                                        <span>复制文字</span>
                                      </>
                                    )}
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => handleAskBuddyWithMemory(mem)}
                                    className="px-2.5 py-1 rounded-full bg-white/[0.06] hover:bg-white/[0.14] text-[11px] text-white/75"
                                  >
                                    问 Buddy
                                  </button>
                                </div>
                                <button
                                  type="button"
                                  onClick={() => void handleDeleteMemory(mem.id)}
                                  className="p-1.5 text-white/35 hover:text-rose-300"
                                  title="删除"
                                >
                                  <Trash2 className="w-3.5 h-3.5" />
                                </button>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )}

                  {((memFilter === "all" && memories.length === 0) ||
                    (memFilter === "images" && imageMemories.length === 0) ||
                    (memFilter === "clipboard" && textMemories.length === 0)) && (
                    <div className="py-12 text-center text-xs text-white/40">
                      暂无相关 7 天记忆
                    </div>
                  )}
                </div>
              </>
            )}

            {vaultSub === "reminders" && (
              <div key="vault-reminders" className="bm-fade-up flex-1 overflow-y-auto space-y-2.5 pb-2">
                <form
                  onSubmit={async (e) => {
                    e.preventDefault();
                    if (!remTitle.trim() || !remTime.trim() || !activeBase || !token) return;
                    const spec = remTime.trim();
                    const m = /^(\d+(?:\.\d+)?)\s*(m|h)?$/i.exec(spec);
                    const body =
                      m && !spec.includes(":")
                        ? {
                            title: remTitle.trim(),
                            delay_minutes:
                              (m[2] || "m").toLowerCase() === "h"
                                ? parseFloat(m[1]) * 60
                                : parseFloat(m[1]),
                          }
                        : { title: remTitle.trim(), at_time: spec };
                    const res = await apiFetch<{ reminder: MobileReminder }>(
                      activeBase,
                      token,
                      "/api/reminders",
                      { method: "POST", body }
                    );
                    setReminders((prev) => [res.reminder, ...prev]);
                    setRemTitle("");
                  }}
                  className="flex items-center gap-1.5"
                >
                  <input
                    type="text"
                    value={remTitle}
                    onChange={(e) => setRemTitle(e.target.value)}
                    placeholder="提醒事项..."
                    className="flex-1 min-w-0 rounded-xl bg-[#111114] border border-white/[0.11] px-3 py-2 text-xs text-white placeholder:text-white/35 outline-none"
                  />
                  <input
                    type="text"
                    value={remTime}
                    onChange={(e) => setRemTime(e.target.value)}
                    placeholder="20m / 18:30"
                    className="w-20 rounded-xl bg-[#111114] border border-white/[0.11] px-2 py-2 text-xs font-mono text-white outline-none"
                  />
                  <button
                    type="submit"
                    disabled={!remTitle.trim()}
                    className="px-3.5 py-2 rounded-xl bg-white text-black disabled:opacity-35 text-xs font-semibold shrink-0"
                  >
                    添加
                  </button>
                </form>

                {reminders.length === 0 ? (
                  <div className="py-12 text-center text-xs text-white/40">暂无提醒</div>
                ) : (
                  reminders.map((r) => (
                    <div
                      key={r.id}
                      className={`rounded-2xl border p-3 flex items-center justify-between gap-2 ${
                        r.enabled
                          ? "bg-[#0e0e11] border-white/[0.09]"
                          : "bg-white/[0.02] border-white/[0.05] opacity-50"
                      }`}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="text-xs font-medium text-white truncate">{r.title}</div>
                        <div className="text-[10.5px] font-mono text-white/50 mt-0.5">
                          {new Date(r.fire_at_ms).toLocaleString()}
                          {r.repeat_rule ? ` · ${r.repeat_rule}` : ""}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => {
                          if (!activeBase || !token) return;
                          void apiFetch(activeBase, token, `/api/reminders/${r.id}`, {
                            method: "DELETE",
                          }).then(() => setReminders((prev) => prev.filter((x) => x.id !== r.id)));
                        }}
                        className="p-1.5 text-white/35 hover:text-rose-300"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>
        )}

        {/* TAB 3: CONNECT / PAIR — Ultra-clean single primary card + collapsed manual fallback */}
        {tab === "connect" && (
          <div
            key="tab-connect"
            className={`flex-1 overflow-y-auto px-4 py-6 ${
              tabDir === "right" ? "bm-tab-right" : "bm-tab-left"
            }`}
          >
            <div className="max-w-md mx-auto w-full space-y-4">
              {/* Primary Hero Card */}
              <div className="rounded-3xl bg-[#0d0d10] border border-white/[0.1] p-5 space-y-4">
                <div className="flex items-center gap-3.5">
                  <div
                    className={`w-11 h-11 rounded-2xl border flex items-center justify-center shrink-0 ${
                      connMode !== "offline"
                        ? "bg-emerald-500/15 border-emerald-400/30 text-emerald-300"
                        : "bg-white/[0.06] border-white/12 text-white/55"
                    }`}
                  >
                    {connMode === "remote" ? (
                      <Cloud className="w-5 h-5" />
                    ) : (
                      <Wifi className="w-5 h-5" />
                    )}
                  </div>
                  <div className="min-w-0">
                    <div className="text-[15px] font-semibold text-white">
                      {connMode === "lan"
                        ? `已连接 · ${pingInfo?.device_name || pairing?.name || "电脑"}`
                        : connMode === "remote"
                        ? `远程已连接 · ${pingInfo?.device_name || pairing?.name || "电脑"}`
                        : "连接你的电脑 Buddy"}
                    </div>
                    <div className="text-xs text-white/45 mt-0.5 truncate">
                      {connMode === "lan"
                        ? "局域网极速直连"
                        : connMode === "remote"
                        ? "远程 5G 加密连接"
                        : "打开电脑 Buddy「设置 → 手机」扫码即可"}
                    </div>
                  </div>
                </div>

                {connError && (
                  <div className="rounded-2xl bg-rose-500/12 border border-rose-400/30 p-3 text-xs text-rose-200 leading-relaxed">
                    {connError}
                  </div>
                )}

                <div className="space-y-2">
                  <button
                    type="button"
                    onClick={() => setShowQrScanner(true)}
                    className="w-full py-3 rounded-2xl bg-white hover:bg-white/90 text-black font-semibold text-sm inline-flex items-center justify-center gap-2 transition"
                  >
                    <QrCode className="w-4 h-4" />
                    <span>{connMode === "offline" ? "扫码连接电脑" : "重新扫码配对"}</span>
                  </button>

                  {pairing && (
                    <button
                      type="button"
                      disabled={connecting}
                      onClick={() => void connectWithConfig(pairing)}
                      className="w-full py-2.5 rounded-2xl bg-white/[0.06] hover:bg-white/[0.11] border border-white/[0.1] text-white/85 text-xs font-medium inline-flex items-center justify-center gap-1.5 disabled:opacity-40 transition"
                    >
                      <RefreshCw className={`w-3.5 h-3.5 ${connecting ? "animate-spin" : ""}`} />
                      <span>{connecting ? "正在连接..." : "重新连接上次电脑"}</span>
                    </button>
                  )}
                </div>
              </div>

              {/* Collapsed Manual Fallback Accordion */}
              <div className="rounded-3xl bg-[#0a0a0d] border border-white/[0.08] overflow-hidden">
                <button
                  type="button"
                  onClick={() => setShowManualConnect((v) => !v)}
                  className="w-full px-4 py-3 flex items-center justify-between text-xs text-white/55 hover:text-white/85 transition"
                >
                  <span>无法扫码？展开手动连接（粘贴配对码 / 填地址）</span>
                  <ChevronDown
                    className={`w-4 h-4 transition-transform ${
                      showManualConnect ? "rotate-180 text-white" : ""
                    }`}
                  />
                </button>

                {showManualConnect && (
                  <div className="bm-sheet-down px-4 pb-4 pt-1 space-y-4 border-t border-white/[0.06]">
                    {/* Option A: Paste Pair Code */}
                    <div className="space-y-2">
                      <div className="text-xs font-medium text-white/85">粘贴配对码</div>
                      <textarea
                        rows={3}
                        value={pairDraft}
                        onChange={(e) => setPairDraft(e.target.value)}
                        placeholder="在电脑 Buddy「设置 → 手机 → 复制配对码」后粘贴到此处..."
                        className="w-full rounded-xl bg-black border border-white/[0.11] p-2.5 text-[11px] font-mono text-white placeholder:text-white/30 outline-none resize-none"
                      />
                      <button
                        type="button"
                        disabled={!pairDraft.trim() || connecting}
                        onClick={() => void handlePairApply(pairDraft)}
                        className="w-full py-2 rounded-xl bg-white/14 hover:bg-white/22 disabled:opacity-35 text-xs font-semibold text-white transition"
                      >
                        导入配对码并连接
                      </button>
                    </div>

                    {/* Option B: Manual Address & Token */}
                    <div className="pt-3 border-t border-white/[0.06] space-y-2">
                      <div className="text-xs font-medium text-white/75">手动填写地址（高级）</div>
                      <input
                        type="text"
                        value={manualLan}
                        onChange={(e) => setManualLan(e.target.value)}
                        placeholder="局域网地址：http://192.168.x.x:19425"
                        className="w-full rounded-xl bg-black border border-white/[0.1] px-3 py-2 text-xs font-mono text-white placeholder:text-white/30 outline-none"
                      />
                      <input
                        type="text"
                        value={manualRemote}
                        onChange={(e) => setManualRemote(e.target.value)}
                        placeholder="远程地址：https://xxx.trycloudflare.com 或 Tailscale IP"
                        className="w-full rounded-xl bg-black border border-white/[0.1] px-3 py-2 text-xs font-mono text-white placeholder:text-white/30 outline-none"
                      />
                      <input
                        type="password"
                        value={manualToken}
                        onChange={(e) => setManualToken(e.target.value)}
                        placeholder="配对 Token"
                        className="w-full rounded-xl bg-black border border-white/[0.1] px-3 py-2 text-xs font-mono text-white placeholder:text-white/30 outline-none"
                      />
                      <button
                        type="button"
                        disabled={!manualToken.trim() || connecting}
                        onClick={() => {
                          const cfg: PairingConfig = {
                            v: 1,
                            name: pairing?.name || "Buddy PC",
                            pairId: pairing?.pairId || "",
                            token: manualToken.trim(),
                            lan: manualLan.trim().replace(/\/$/, ""),
                            remote: manualRemote.trim().replace(/\/$/, ""),
                            dir:
                              pairing?.dir || "https://buddy-companion.marcus-dev00.workers.dev",
                          };
                          void connectWithConfig(cfg).then((ok) => {
                            if (ok) setTab("chat");
                          });
                        }}
                        className="w-full py-2 rounded-xl bg-white/14 hover:bg-white/22 disabled:opacity-35 text-xs font-semibold text-white transition"
                      >
                        保存并测试连接
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Native Android APK Version & Update Check (hidden on Web PWA) */}
              {localVersion.isNativeApk && (
                <div className="rounded-3xl bg-[#0a0a0d] border border-white/[0.08] px-4 py-3 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <div className="text-xs text-white/55 font-mono">
                      BuddyMobile v{localVersion.name}
                    </div>
                    {availableUpdate ? (
                      <button
                        type="button"
                        onClick={() => triggerApkUpdate(availableUpdate.apkUrl)}
                        className="px-3 py-1 rounded-full bg-white text-black text-xs font-semibold inline-flex items-center gap-1"
                      >
                        <Download className="w-3 h-3" />
                        <span>更新 v{availableUpdate.versionName}</span>
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={checkingUpdate}
                        onClick={() => void handleManualCheckUpdate()}
                        className="px-3 py-1 rounded-full bg-white/[0.07] border border-white/[0.1] text-white/75 text-xs inline-flex items-center gap-1 disabled:opacity-45"
                      >
                        {checkingUpdate ? (
                          <Loader2 className="w-3 h-3 animate-spin" />
                        ) : (
                          <RefreshCw className="w-3 h-3" />
                        )}
                        <span>检查更新</span>
                      </button>
                    )}
                  </div>
                  {updateStatusMsg && (
                    <div className="text-[11px] text-white/45">{updateStatusMsg}</div>
                  )}
                </div>
              )}
            </div>
          </div>
        )}
      </main>

      {/* BOTTOM NAVIGATION BAR — Pure Black 3-Tab Minimalist Dock with Sliding Pill */}
      <nav className="safe-bottom px-3 pt-1.5 border-t border-white/[0.08] bg-black/95 backdrop-blur-xl shrink-0">
        <div className="relative max-w-md mx-auto grid grid-cols-3">
          {/* Smooth sliding glass indicator */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute top-0 bottom-0 left-0 w-1/3 p-0.5 transition-transform duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]"
            style={{
              transform: `translate3d(${TAB_ORDER.indexOf(tab) * 100}%, 0, 0)`,
            }}
          >
            <div className="w-full h-full rounded-2xl bg-white/[0.12] border border-white/[0.16]" />
          </div>
          {(
            [
              { id: "chat", label: "对话", Icon: MessageSquare },
              { id: "vault", label: "记忆", Icon: Clock },
              { id: "connect", label: "连接", Icon: Link2 },
            ] as const
          ).map(({ id, label, Icon }) => {
            const active = tab === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => switchTab(id)}
                className={`relative z-10 py-1.5 rounded-2xl flex flex-col items-center justify-center gap-0.5 active:scale-95 transition-all duration-200 ${
                  active ? "text-white" : "text-white/42 hover:text-white/75"
                }`}
              >
                <Icon
                  className={`w-4 h-4 transition-transform duration-200 ${
                    active ? "scale-110" : "scale-100"
                  }`}
                />
                <span className="text-[10.5px] font-medium">{label}</span>
              </button>
            );
          })}
        </div>
      </nav>

      {/* FULLSCREEN SCREENSHOT LIGHTBOX (7-Day Memory Image Viewer) */}
      {lightboxMem && activeBase && token && (
        <div
          className="bm-modal-backdrop fixed inset-0 z-50 bg-black/95 backdrop-blur-md flex flex-col"
          onClick={() => setLightboxMem(null)}
        >
          <div
            className="safe-top px-4 pb-3 flex items-center justify-between gap-2 border-b border-white/[0.08]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="min-w-0">
              <div className="text-xs font-medium text-white truncate">
                {lightboxMem.app_name || "截图"}
              </div>
              <div className="text-[10px] text-white/45">
                {new Date(lightboxMem.updated_at).toLocaleString()}
              </div>
            </div>
            <button
              type="button"
              onClick={() => setLightboxMem(null)}
              className="w-8 h-8 rounded-full bg-white/10 active:scale-90 text-white flex items-center justify-center shrink-0 transition"
            >
              <X className="w-4 h-4" />
            </button>
          </div>

          <div
            className="flex-1 min-h-0 flex items-center justify-center p-3 overflow-auto"
            onClick={(e) => e.stopPropagation()}
          >
            <img
              src={buildMemoryImageUrl(activeBase, token, lightboxMem.id, true)}
              alt={lightboxMem.app_name || "截图大图"}
              className="bm-modal-pop max-w-full max-h-full object-contain rounded-xl"
            />
          </div>

          <div
            className="safe-bottom px-4 pt-2.5 border-t border-white/[0.08] flex items-center justify-between gap-2"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => handleAskBuddyWithMemory(lightboxMem)}
                className="px-3.5 py-2 rounded-full bg-white text-black text-xs font-semibold active:scale-95 transition"
              >
                发给 Buddy 分析
              </button>
              <button
                type="button"
                onClick={() =>
                  handleSaveFile(
                    buildMemoryImageUrl(activeBase, token, lightboxMem.id, true, true),
                    `buddy_mem_${lightboxMem.id}.png`
                  )
                }
                className="px-3 py-2 rounded-full bg-white/10 text-white text-xs inline-flex items-center gap-1 active:scale-95 transition"
              >
                <Download className="w-3.5 h-3.5" />
                <span>保存</span>
              </button>
            </div>
            <button
              type="button"
              onClick={() => void handleDeleteMemory(lightboxMem.id)}
              className="p-2 rounded-full bg-rose-500/18 text-rose-300 active:scale-90 transition"
              title="删除"
            >
              <Trash2 className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      {/* ON-DEMAND PC DESKTOP PEEK MODAL */}
      {showDesktopPeek && (
        <div
          className="bm-modal-backdrop fixed inset-0 z-50 bg-black/95 backdrop-blur-md flex flex-col"
          onClick={() => setShowDesktopPeek(false)}
        >
          <div
            className="safe-top px-4 pb-3 flex items-center justify-between gap-2 border-b border-white/[0.08]"
            onClick={(e) => e.stopPropagation()}
          >
            <div>
              <div className="text-xs font-semibold text-white">电脑当前屏幕</div>
              <div className="text-[10px] text-white/45">
                {desktopTs ? `抓拍于 ${new Date(desktopTs).toLocaleTimeString()}` : "正在抓拍..."}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={grabbingDesktop}
                onClick={() => void handleGrabDesktop()}
                className="px-3 py-1.5 rounded-full bg-white text-black text-xs font-semibold inline-flex items-center gap-1 disabled:opacity-50 active:scale-95 transition"
              >
                {grabbingDesktop ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Camera className="w-3.5 h-3.5" />
                )}
                <span>刷新抓拍</span>
              </button>
              <button
                type="button"
                onClick={() => setShowDesktopPeek(false)}
                className="w-8 h-8 rounded-full bg-white/10 text-white flex items-center justify-center active:scale-90 transition"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
          </div>

          <div
            className="flex-1 min-h-0 flex items-center justify-center p-3 overflow-auto"
            onClick={(e) => e.stopPropagation()}
          >
            {desktopFrame ? (
              <img
                src={desktopFrame}
                alt="PC Desktop"
                className="bm-modal-pop max-w-full max-h-full object-contain rounded-xl border border-white/10"
              />
            ) : (
              <div className="flex items-center gap-2 text-xs text-white/50">
                <Loader2 className="w-4 h-4 animate-spin" />
                <span>正在抓拍电脑屏幕...</span>
              </div>
            )}
          </div>
        </div>
      )}

      {showQrScanner && (
        <QrScannerModal
          onScanned={(raw) => {
            setShowQrScanner(false);
            void handlePairApply(raw);
          }}
          onClose={() => setShowQrScanner(false)}
        />
      )}
    </div>
  );
}
