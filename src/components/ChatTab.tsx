import React, { useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import { invoke } from "@tauri-apps/api/core";
import {
  Bot,
  Brain,
  Check,
  Copy,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Download,
  FileText,
  Film,
  Folder,
  FolderOpen,
  History,
  ImageDown,
  Loader2,
  Lock,
  MessageSquare,
  Monitor,
  Music,
  Paperclip,
  Pause,
  Pin,
  Play,
  Plus,
  RefreshCw,
  ScanText,
  Search,
  ShieldAlert,
  Sparkles,
  Square,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import { formatRelativeTime, getStrings } from "../i18n";
import {
  ALL_THINKING_LEVELS,
  formatThinkingBadge,
  type ModelThinkingCapability,
} from "../thinking";
import type {
  AgentStep,
  ApprovalDecision,
  AttachmentPayload,
  ChatMessage,
  ConversationSummary,
  MemoryItem,
  ModelProfile,
  PluginProgressPayload,
  ShareStatePayload,
  ShareTargetInfo,
  SkillInfo,
  ThinkingLevel,
  UiLang,
} from "../types";

function extractNodeText(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(extractNodeText).join("");
  if (node && typeof node === "object" && "props" in node) {
    const props = (node as { props?: { children?: React.ReactNode } }).props;
    return extractNodeText(props?.children);
  }
  return "";
}

function MarkdownCodePre({
  children,
  copyLabel,
  copiedLabel,
}: {
  children?: React.ReactNode;
  copyLabel: string;
  copiedLabel: string;
}) {
  const [copied, setCopied] = useState(false);
  const firstChild = Array.isArray(children) ? children[0] : children;
  const childClass =
    firstChild && typeof firstChild === "object" && "props" in firstChild
      ? String((firstChild as { props?: { className?: string } }).props?.className ?? "")
      : "";
  const langMatch = /language-([\w-]+)/.exec(childClass);
  const codeText = extractNodeText(children).replace(/\n$/, "");

  return (
    <div className="my-1.5 rounded-lg border border-white/10 bg-black/55 overflow-hidden">
      <div className="flex items-center justify-between px-2.5 py-1 bg-white/[0.04] border-b border-white/[0.06] text-[10px] font-mono text-white/50">
        <span>{langMatch ? langMatch[1] : "code"}</span>
        <button
          type="button"
          onClick={() => {
            void navigator.clipboard.writeText(codeText).then(() => {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1400);
            });
          }}
          className="inline-flex items-center gap-1 text-white/65 hover:text-white transition"
        >
          {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
          <span>{copied ? copiedLabel : copyLabel}</span>
        </button>
      </div>
      <pre className="p-2 overflow-x-auto m-0 bg-transparent text-[11px] font-mono leading-relaxed">
        {children}
      </pre>
    </div>
  );
}

interface ChatTabProps {
  lang: UiLang;
  agentMode: boolean;
  thinkingLevel: ThinkingLevel;
  thinkingCapability?: ModelThinkingCapability;
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
  onRemoveAttachment: (idx: number) => void;
  onSend: (promptOverride?: string, attOverride?: AttachmentPayload[]) => void;
  onStopStream: () => void;
  onToggleAgentMode: () => void;
  onChangeThinkingLevel: (level: ThinkingLevel) => void;
  onResolveApproval: (stepId: string, decision: ApprovalDecision) => void;
  onNewChat: () => void;
  onSelectConversation: (id: number) => void;
  onDeleteConversation: (id: number) => void;
  onOpenSettings: () => void;
  enabledPlugins?: Record<string, boolean>;
  installedPlugins?: Record<string, boolean>;
  skills?: SkillInfo[];
  onOpenPlugin?: (pluginId: string, paths?: string[]) => void;
  onQuickOcr?: (source: string) => void;
  onQuickDownloadUrl?: (url: string, format: string) => void;
  pluginProgress?: Record<string, PluginProgressPayload>;
  shareState?: ShareStatePayload | null;
  modelProfiles?: ModelProfile[];
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

function AttachmentIcon({ kind }: { kind: AttachmentPayload["kind"] }) {
  if (kind === "folder") return <Folder className="w-3 h-3 text-amber-300 shrink-0" />;
  if (kind === "file") return <FileText className="w-3 h-3 text-emerald-300 shrink-0" />;
  return <Paperclip className="w-3 h-3 shrink-0" />;
}

function AgentStepCard({
  step,
  lang,
  onResolve,
}: {
  step: AgentStep;
  lang: UiLang;
  onResolve: (stepId: string, decision: ApprovalDecision) => void;
}) {
  const [expanded, setExpanded] = useState(step.status === "waiting_approval");
  const s = getStrings(lang).chat;

  if (step.status === "waiting_approval") {
    const allowAll = step.detail === "allow_all" && !step.is_destructive;
    return (
      <div className="rounded-xl bg-amber-500/10 border border-amber-400/35 p-2 space-y-1.5">
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-start gap-1.5 min-w-0">
            <ShieldAlert className="w-3.5 h-3.5 text-amber-300 shrink-0 mt-0.5" />
            <div className="min-w-0">
              <div className="text-[11px] font-medium text-amber-200 leading-snug break-all">
                {step.summary}
              </div>
              <div className="text-[10px] text-amber-300/75">
                {step.is_destructive ? s.destructiveWarning : s.approvalRequired}
              </div>
            </div>
          </div>
          <span className="px-1.5 py-0.5 rounded bg-black/35 font-mono text-[9.5px] text-amber-300 shrink-0">
            {step.tool_name}
          </span>
        </div>

        {step.preview && (
          <pre className="max-h-24 overflow-y-auto rounded-lg bg-black/55 border border-white/10 px-2 py-1.5 text-[10px] font-mono text-white/80 whitespace-pre-wrap break-all">
            {step.preview}
          </pre>
        )}

        <div className="flex items-center justify-end gap-1.5 pt-0.5">
          <button
            type="button"
            onClick={() => onResolve(step.step_id, "reject")}
            className="px-2 py-1 rounded-lg bg-white/10 hover:bg-rose-500/30 text-[10.5px] text-white/85 flex items-center gap-1 transition"
          >
            <X className="w-3 h-3" />
            <span>{s.reject}</span>
          </button>
          {allowAll && (
            <button
              type="button"
              onClick={() => onResolve(step.step_id, "approve_all_turn")}
              className="px-2 py-1 rounded-lg bg-sky-500/20 hover:bg-sky-500/35 border border-sky-400/35 text-[10.5px] text-sky-200 transition"
            >
              {s.approveAll}
            </button>
          )}
          <button
            type="button"
            onClick={() => onResolve(step.step_id, "approve_once")}
            className="px-2.5 py-1 rounded-lg bg-emerald-500 hover:bg-emerald-400 text-slate-950 font-semibold text-[10.5px] flex items-center gap-1 transition"
          >
            <Check className="w-3 h-3" />
            <span>{s.approve}</span>
          </button>
        </div>
      </div>
    );
  }

  const icon =
    step.status === "running" ? (
      <Loader2 className="w-3 h-3 text-sky-300 animate-spin shrink-0" />
    ) : step.status === "done" ? (
      <CheckCircle2 className="w-3 h-3 text-emerald-400 shrink-0" />
    ) : step.status === "rejected" ? (
      <XCircle className="w-3 h-3 text-amber-300/80 shrink-0" />
    ) : (
      <XCircle className="w-3 h-3 text-rose-400 shrink-0" />
    );

  const hasDetail = Boolean(step.preview || (step.detail && step.detail !== "allow_all"));

  return (
    <div className="rounded-lg bg-black/30 border border-white/[0.08] px-2 py-1 text-[10.5px]">
      <div
        onClick={() => {
          if (hasDetail) setExpanded((v) => !v);
        }}
        className={`flex items-center justify-between gap-1.5 ${
          hasDetail ? "cursor-pointer" : ""
        }`}
      >
        <div className="flex items-center gap-1.5 min-w-0 flex-1">
          {icon}
          <span className="truncate text-white/80 font-mono text-[10.5px]">
            {step.summary}
          </span>
        </div>
        <div className="flex items-center gap-1 shrink-0 text-[10px] text-white/45">
          {step.status === "rejected" ? (
            <span className="text-amber-300/80">{s.rejectedLabel}</span>
          ) : (
            step.detail &&
            step.detail !== "allow_all" && (
              <span className="max-w-[85px] truncate">{step.detail}</span>
            )
          )}
          {hasDetail &&
            (expanded ? (
              <ChevronDown className="w-2.5 h-2.5" />
            ) : (
              <ChevronRight className="w-2.5 h-2.5" />
            ))}
        </div>
      </div>
      {expanded && hasDetail && (
        <div className="mt-1 pt-1 border-t border-white/[0.06] text-[10px] font-mono text-white/65 whitespace-pre-wrap break-all max-h-20 overflow-y-auto">
          {step.preview || step.detail}
        </div>
      )}
    </div>
  );
}

function ThoughtBox({
  thought,
  isLive,
  lang,
}: {
  thought?: string;
  isLive: boolean;
  lang: UiLang;
}) {
  const [open, setOpen] = useState(false);
  const s = getStrings(lang).chat;

  return (
    <div className="mb-1">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-center gap-1.5 py-0.5 text-[11px] text-white/45 hover:text-white/80 transition cursor-pointer select-none"
      >
        {isLive ? (
          <span className="w-1.5 h-1.5 rounded-full bg-white/75 animate-pulse shrink-0" />
        ) : (
          <Brain className="w-3 h-3 text-white/40 shrink-0" />
        )}
        <span className={isLive ? "text-white/65 animate-pulse" : ""}>
          {isLive ? s.thinkingActive : s.thoughtSaved}
        </span>
        {(thought?.trim() || open) &&
          (open ? (
            <ChevronDown className="w-2.5 h-2.5 text-white/40 shrink-0" />
          ) : (
            <ChevronRight className="w-2.5 h-2.5 text-white/40 shrink-0" />
          ))}
      </button>
      {open && (
        <div className="mt-1 mb-1 pl-2.5 border-l border-white/15 text-[11px] text-white/55 whitespace-pre-wrap break-words max-h-36 overflow-y-auto leading-relaxed">
          {thought?.trim() ? thought : "..."}
        </div>
      )}
    </div>
  );
}

export const THINKING_STEPS: ThinkingLevel[] = ALL_THINKING_LEVELS;

const DEFAULT_THINKING_CAP: ModelThinkingCapability = {
  levels: ALL_THINKING_LEVELS,
  isNative: false,
  familyName: "Prompt",
};

export function getStepDisplay(
  step: ThinkingLevel,
  lang: UiLang,
  cap: ModelThinkingCapability = DEFAULT_THINKING_CAP
): { short: string; label: string; desc: string } {
  const s = getStrings(lang).chat;
  const base = s.thinkLevels[step];
  const ov = cap.stepOverrides?.[step];
  if (!ov) return base;
  return {
    short: ov.short,
    label: ov.label,
    desc: lang === "zh" ? ov.descZh : ov.descEn,
  };
}

export function ThinkingSlider({
  value,
  lang,
  capability,
  disabled,
  compact,
  onChange,
}: {
  value: ThinkingLevel;
  lang: UiLang;
  capability?: ModelThinkingCapability;
  disabled?: boolean;
  compact?: boolean;
  onChange: (next: ThinkingLevel) => void;
}) {
  const s = getStrings(lang).chat;
  const cap = capability ?? DEFAULT_THINKING_CAP;
  const steps = cap.levels.length > 0 ? cap.levels : THINKING_STEPS;
  const rawIdx = steps.indexOf(value);
  const idx = rawIdx >= 0 ? rawIdx : 0;
  const count = steps.length;
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const pickByClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.max(0, Math.min(0.999, (clientX - rect.left) / rect.width));
    const nextIdx = Math.floor(ratio * count);
    const next = steps[nextIdx];
    if (next && next !== value) {
      onChange(next);
    }
  };

  const activeStep = steps[idx] ?? "medium";
  const accentBar =
    activeStep === "off"
      ? "bg-white/25"
      : activeStep === "low"
      ? "bg-white/60"
      : activeStep === "medium"
      ? "bg-sky-400"
      : activeStep === "high"
      ? "bg-indigo-400"
      : "bg-amber-400 shadow-[0_0_8px_rgba(251,191,36,0.75)]";

  const widthClass = compact
    ? count <= 2
      ? "w-[82px] h-[24px]"
      : count === 3
      ? "w-[110px] h-[24px]"
      : count === 4
      ? "w-[128px] h-[24px]"
      : "w-[144px] h-[24px]"
    : count <= 2
    ? "w-[116px] h-[26px]"
    : count === 3
    ? "w-[148px] h-[26px]"
    : count === 4
    ? "w-[174px] h-[26px]"
    : "w-[196px] h-[26px]";

  const badgeText = formatThinkingBadge(cap, lang);

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label={s.thinkPrefix}
      aria-valuemin={0}
      aria-valuemax={Math.max(0, count - 1)}
      aria-valuenow={idx}
      onPointerDown={(e) => {
        if (disabled) return;
        draggingRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        pickByClientX(e.clientX);
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current || disabled) return;
        pickByClientX(e.clientX);
      }}
      onPointerUp={(e) => {
        if (draggingRef.current) {
          draggingRef.current = false;
          try {
            e.currentTarget.releasePointerCapture(e.pointerId);
          } catch {
            // ignore
          }
        }
      }}
      onPointerCancel={() => {
        draggingRef.current = false;
      }}
      style={{ gridTemplateColumns: `repeat(${count}, minmax(0, 1fr))` }}
      className={`relative grid items-center rounded-lg bg-[#08090d] border border-white/[0.09] p-[2px] select-none touch-none ${widthClass} ${
        disabled ? "opacity-45 pointer-events-none" : "cursor-pointer"
      }`}
    >
      {/* Smooth Gliding Elevated Thumb (Codex / Claude Code matte glass) */}
      <div
        className="pointer-events-none absolute top-[2px] bottom-[2px] left-[2px] rounded-[6px] bg-white/[0.11] border border-white/[0.15] shadow-[0_1px_4px_rgba(0,0,0,0.65)] flex flex-col justify-end items-center pb-[1.5px] transition-transform duration-200 ease-out"
        style={{
          width: `calc((100% - 4px) / ${count})`,
          transform: `translateX(${idx * 100}%)`,
        }}
      >
        <span className={`w-3.5 h-[1.5px] rounded-full transition-colors ${accentBar}`} />
      </div>

      {steps.map((step, i) => {
        const active = i === idx;
        const stepInfo = getStepDisplay(step, lang, cap);
        return (
          <div
            key={step}
            title={`${badgeText} — ${stepInfo.desc}`}
            className={`relative z-10 text-center text-[9.5px] tracking-tight transition-colors ${
              active
                ? step === "max"
                  ? "text-amber-200 font-semibold"
                  : "text-white font-medium"
                : "text-white/40 hover:text-white/75"
            }`}
          >
            {stepInfo.short}
          </div>
        );
      })}
    </div>
  );
}

export default function ChatTab({
  lang,
  agentMode,
  thinkingLevel,
  thinkingCapability,
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
  onRemoveAttachment,
  onSend,
  onStopStream,
  onToggleAgentMode,
  onChangeThinkingLevel,
  onResolveApproval,
  onNewChat,
  onSelectConversation,
  onDeleteConversation,
  onOpenSettings,
  enabledPlugins = {},
  installedPlugins = {},
  skills = [],
  onOpenPlugin,
  onQuickOcr,
  onQuickDownloadUrl,
  pluginProgress = {},
  shareState = null,
  modelProfiles = [],
  defaultWatchProfileId = "",
}: ChatTabProps) {
  const [showHistory, setShowHistory] = useState(false);
  const [historySearch, setHistorySearch] = useState("");
  const [showSharePanel, setShowSharePanel] = useState(false);
  const [shareTargets, setShareTargets] = useState<ShareTargetInfo[]>([]);
  const [selectedTargetId, setSelectedTargetId] = useState("auto_cursor");
  const [shareGoal, setShareGoal] = useState("");
  const [shareCadence, setShareCadence] = useState<"quiet" | "balanced" | "active">("balanced");
  const [shareShowBorder, setShareShowBorder] = useState(true);
  const [shareWatchId, setShareWatchId] = useState(defaultWatchProfileId || "");
  const [shareError, setShareError] = useState<string | null>(null);
  const [shareStarting, setShareStarting] = useState(false);
  const [activeUserIdx, setActiveUserIdx] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [slashSelIdx, setSlashSelIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const scrollBoxRef = useRef<HTMLDivElement | null>(null);
  const msgRefs = useRef<Record<number, HTMLDivElement | null>>({});
  const s = getStrings(lang).chat;
  const sp = getStrings(lang).plugins;

  useEffect(() => {
    setShareWatchId((cur) => cur || defaultWatchProfileId || "");
  }, [defaultWatchProfileId]);

  useEffect(() => {
    const onFocusInput = () => {
      inputRef.current?.focus();
    };
    window.addEventListener("buddy-focus-chat-input", onFocusInput);
    return () => window.removeEventListener("buddy-focus-chat-input", onFocusInput);
  }, []);

  useEffect(() => {
    if (shareState?.active) {
      setSelectedTargetId(shareState.target_id || "auto_cursor");
      setShareGoal(shareState.goal || "");
      setShareCadence((shareState.cadence as "quiet" | "balanced" | "active") || "balanced");
      setShareShowBorder(shareState.show_border);
      if (shareState.watch_profile_id) {
        setShareWatchId(shareState.watch_profile_id);
      }
    }
  }, [shareState?.active, shareState?.target_id]);

  const loadShareTargets = async () => {
    try {
      const list = await invoke<ShareTargetInfo[]>("list_share_targets");
      setShareTargets(list);
    } catch {
      // ignore
    }
  };

  const handleOpenSharePanel = () => {
    const next = !showSharePanel;
    setShowSharePanel(next);
    if (next) {
      setShowHistory(false);
      setShareError(null);
      void loadShareTargets();
    }
  };

  const handleStartShare = async () => {
    setShareStarting(true);
    setShareError(null);
    try {
      await invoke("start_screen_share", {
        request: {
          target_id: selectedTargetId,
          goal: shareGoal,
          cadence: shareCadence,
          show_border: shareShowBorder,
          watch_profile_id: shareWatchId,
          window_edge: "top",
          window_ratio: 0.5,
        },
      });
      setShowSharePanel(false);
    } catch (err) {
      setShareError(String(err));
    } finally {
      setShareStarting(false);
    }
  };

  const syncLiveShareOpt = (patch: {
    goal?: string;
    cadence?: string;
    showBorder?: boolean;
    watchProfileId?: string;
  }) => {
    if (!shareState?.active) return;
    void invoke("update_screen_share_options", patch).catch(() => {});
  };

  useEffect(() => {
    if (!shareState?.active) return;
    if (shareGoal === (shareState.goal || "")) return;
    const t = window.setTimeout(() => {
      void invoke("update_screen_share_options", { goal: shareGoal }).catch(() => {});
    }, 350);
    return () => window.clearTimeout(t);
  }, [shareGoal, shareState?.active, shareState?.goal]);

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
    <div className="flex-1 flex flex-col min-h-0">
      {/* Top Bar: History Toggle + Thinking Effort + New Chat */}
      <div className="flex items-center justify-between gap-1 pb-1.5 shrink-0 border-b border-white/[0.06] mb-1.5">
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
          <span className="truncate max-w-[60px]">
            {showHistory
              ? s.backToChat
              : activeConv && activeConv.message_count > 0
              ? activeConv.title
              : `${s.history} (${conversations.filter((c) => c.message_count > 0).length})`}
          </span>
        </button>

        <div className="flex items-center gap-1 shrink-0">
          {!agentMode && (
            <button
              type="button"
              disabled={isThinking}
              onClick={onToggleAgentMode}
              title={s.agentToggleTip}
              className="flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-medium border bg-white/[0.05] border-white/10 text-white/45 hover:text-white/85 transition disabled:opacity-50"
            >
              <Bot className="w-2.5 h-2.5" />
              <span>{s.agentOff}</span>
            </button>
          )}

          <button
            type="button"
            onClick={handleOpenSharePanel}
            title={s.shareTitle}
            className={`flex items-center gap-1 px-1.5 py-0.5 rounded-lg text-[10.5px] border transition ${
              shareState?.active
                ? "bg-white/15 border-white/25 text-white"
                : showSharePanel
                ? "bg-white/15 border-white/20 text-white"
                : "border-transparent text-white/60 hover:text-white hover:bg-white/[0.07]"
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
              <Monitor className="w-3 h-3 shrink-0" />
            )}
            <span>{shareState?.active ? (shareState.paused ? s.sharePaused : s.shareLive) : s.shareBtn}</span>
          </button>

          <ThinkingSlider
            compact
            value={thinkingLevel}
            capability={thinkingCapability}
            lang={lang}
            disabled={isThinking}
            onChange={onChangeThinkingLevel}
          />

          <button
            onClick={() => {
              setShowHistory(false);
              setShowSharePanel(false);
              onNewChat();
            }}
            disabled={isThinking}
            title={s.newChatTip}
            className="flex items-center gap-1 px-1.5 py-0.5 rounded-lg text-[11px] text-sky-300 hover:bg-sky-500/15 disabled:opacity-40 transition"
          >
            <Plus className="w-3 h-3" />
            <span>{s.newChat}</span>
          </button>
        </div>
      </div>

      {/* Screen Share Launcher & Live Controls Panel */}
      {showSharePanel ? (
        <div className="flex-1 flex flex-col min-h-0 gap-2">
          <div className="flex-1 overflow-y-auto pr-0.5 space-y-2">
            {/* Target Selection */}
            <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2 space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="text-[10.5px] font-medium text-white/75">{s.shareTarget}</span>
                <button
                  type="button"
                  onClick={() => void loadShareTargets()}
                  className="text-[10px] text-white/55 hover:text-white inline-flex items-center gap-1"
                >
                  <RefreshCw className="w-2.5 h-2.5" />
                  <span>{s.shareRefresh}</span>
                </button>
              </div>

              <div className="space-y-1 max-h-28 overflow-y-auto pr-0.5">
                {shareTargets.map((t) => {
                  const chosen = selectedTargetId === t.id;
                  return (
                    <button
                      key={t.id}
                      type="button"
                      onClick={() => setSelectedTargetId(t.id)}
                      className={`w-full text-left px-2 py-1 rounded-lg border text-[11px] flex items-center justify-between gap-2 transition ${
                        chosen
                          ? "bg-white/15 border-white/35 text-white"
                          : "bg-black/25 border-white/[0.06] text-white/70 hover:text-white hover:bg-white/[0.07]"
                      }`}
                    >
                      <div className="min-w-0 flex items-center gap-1.5">
                        <span className="text-[9px] font-mono uppercase px-1 py-0.2 rounded bg-white/10 text-white/65 shrink-0">
                          {t.kind === "window" ? t.app_name || "APP" : "SCR"}
                        </span>
                        <span className="truncate">
                          {t.id === "auto_cursor" ? s.shareMouseScreen : t.title}
                        </span>
                      </div>
                      <span className="text-[9.5px] font-mono text-white/40 shrink-0">
                        {t.width}×{t.height}
                      </span>
                    </button>
                  );
                })}
              </div>

              {shareState?.active && shareState.target_kind === "window" && (
                <div className="pt-1 border-t border-white/[0.06] flex items-center gap-1.5">
                  <button
                    type="button"
                    onClick={() =>
                      void invoke("set_window_dock_preset", { edge: "top", ratio: 0.15 }).catch(() => {})
                    }
                    className="flex-1 py-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-[10px] text-white/80"
                  >
                    {s.shareSnapTopLeft}
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      void invoke("set_window_dock_preset", { edge: "top", ratio: 0.5 }).catch(() => {})
                    }
                    className="flex-1 py-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-[10px] text-white/80"
                  >
                    {s.shareSnapTopCenter}
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      void invoke("set_window_dock_preset", { edge: "top", ratio: 0.85 }).catch(() => {})
                    }
                    className="flex-1 py-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-[10px] text-white/80"
                  >
                    {s.shareSnapTopRight}
                  </button>
                </div>
              )}
            </div>

            {/* Watch Vision Model + Co-Work Goal */}
            <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2 space-y-1.5">
              <div className="flex items-center justify-between gap-2">
                <label className="text-[10.5px] text-white/65 shrink-0">{s.shareWatchModel}</label>
                <select
                  value={shareWatchId || ""}
                  onChange={(e) => {
                    setShareWatchId(e.target.value);
                    syncLiveShareOpt({ watchProfileId: e.target.value });
                  }}
                  className="max-w-[190px] rounded-lg bg-[#111319] border border-white/15 px-2 py-0.5 text-[10.5px] text-white outline-none"
                >
                  <option value="">{s.shareFollowChat}</option>
                  {modelProfiles.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name || p.model_id}
                      {p.supports_vision ? "" : " (Text)"}
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-1">
                <label className="text-[10.5px] text-white/65">{s.shareGoal}</label>
                <input
                  type="text"
                  value={shareGoal}
                  onChange={(e) => {
                    setShareGoal(e.target.value);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      syncLiveShareOpt({ goal: shareGoal });
                      (e.currentTarget as HTMLInputElement).blur();
                    }
                  }}
                  onBlur={() => syncLiveShareOpt({ goal: shareGoal })}
                  placeholder={s.shareGoalPlaceholder}
                  className="w-full rounded-lg bg-black/30 border border-white/10 focus:border-white/30 outline-none px-2 py-1 text-[11px] text-white placeholder:text-white/30"
                />
                <div className="flex flex-wrap gap-1">
                  {s.shareGoalPresets.map((preset) => (
                    <button
                      key={preset}
                      type="button"
                      onClick={() => {
                        setShareGoal(preset);
                        syncLiveShareOpt({ goal: preset });
                      }}
                      className="px-2 py-0.5 rounded-full bg-white/[0.06] hover:bg-white/[0.14] text-[9.5px] text-white/75 transition"
                    >
                      {preset}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            {/* Cadence & Border */}
            <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2 space-y-1.5">
              <div className="grid grid-cols-3 gap-1">
                {(["quiet", "balanced", "active"] as const).map((cad) => {
                  const active = shareCadence === cad;
                  const meta = s.shareCadences[cad];
                  return (
                    <button
                      key={cad}
                      type="button"
                      title={meta.desc}
                      onClick={() => {
                        setShareCadence(cad);
                        syncLiveShareOpt({ cadence: cad });
                      }}
                      className={`py-1 px-1.5 rounded-lg border text-[10.5px] transition ${
                        active
                          ? "bg-white/15 border-white/35 text-white font-medium"
                          : "bg-black/20 border-white/[0.06] text-white/55 hover:text-white"
                      }`}
                    >
                      {meta.label}
                    </button>
                  );
                })}
              </div>
              <p className="text-[10px] text-white/45 px-0.5">
                {s.shareCadences[shareCadence].desc}
              </p>

              <label className="flex items-center justify-between gap-2 pt-0.5 cursor-pointer select-none">
                <span className="text-[10.5px] text-white/75">{s.shareShowBorder}</span>
                <input
                  type="checkbox"
                  checked={shareShowBorder}
                  onChange={(e) => {
                    setShareShowBorder(e.target.checked);
                    syncLiveShareOpt({ showBorder: e.target.checked });
                  }}
                  className="accent-white w-3.5 h-3.5 rounded"
                />
              </label>
            </div>

            {shareError && (
              <div className="text-[10.5px] text-rose-300 bg-rose-500/10 border border-rose-400/25 rounded-lg px-2 py-1">
                {shareError}
              </div>
            )}
          </div>

          <div className="pt-1 border-t border-white/[0.08] flex items-center justify-between gap-2 shrink-0">
            <span className="text-[9.5px] text-white/40 truncate">{s.shareHotkeyHint}</span>
            <div className="flex items-center gap-1.5 shrink-0">
              {shareState?.active ? (
                <>
                  <button
                    type="button"
                    onClick={() => void invoke("toggle_screen_share_pause").catch(() => {})}
                    className="px-2.5 py-1 rounded-lg bg-white/[0.08] hover:bg-white/[0.16] border border-white/10 text-[11px] text-white flex items-center gap-1"
                  >
                    {shareState.paused ? (
                      <>
                        <Play className="w-3 h-3" />
                        <span>{s.shareResume}</span>
                      </>
                    ) : (
                      <>
                        <Pause className="w-3 h-3" />
                        <span>{s.sharePause}</span>
                      </>
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      void invoke("stop_screen_share").catch(() => {});
                      setShowSharePanel(false);
                    }}
                    className="px-2.5 py-1 rounded-lg bg-rose-500/20 hover:bg-rose-500/35 border border-rose-400/30 text-[11px] text-rose-200 flex items-center gap-1"
                  >
                    <Square className="w-2.5 h-2.5 fill-current" />
                    <span>{s.shareStop}</span>
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  disabled={shareStarting}
                  onClick={() => void handleStartShare()}
                  className="px-3 py-1 rounded-lg bg-white hover:bg-white/90 text-black font-semibold text-[11px] flex items-center gap-1.5 transition"
                >
                  {shareStarting ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <span className="w-2 h-2 rounded-full bg-red-500" />
                  )}
                  <span>{s.shareStart}</span>
                </button>
              )}
            </div>
          </div>
        </div>
      ) : /* Redesigned Past Conversations Panel */ showHistory ? (
        <div className="flex-1 flex flex-col min-h-0 gap-2">
          <div className="relative shrink-0">
            <Search className="w-3 h-3 text-white/35 absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={historySearch}
              onChange={(e) => setHistorySearch(e.target.value)}
              placeholder={s.searchHistory}
              className="w-full rounded-xl bg-white/[0.05] border border-white/10 focus:border-sky-400/45 outline-none pl-7 pr-7 py-1 text-[11px] text-white placeholder:text-white/35"
            />
            {historySearch && (
              <button
                type="button"
                onClick={() => setHistorySearch("")}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-white/40 hover:text-white"
              >
                <X className="w-3 h-3" />
              </button>
            )}
          </div>

          <div className="flex-1 overflow-y-auto space-y-2.5 pr-1">
            {(() => {
              const q = historySearch.trim().toLowerCase();
              const filtered = conversations.filter((c) =>
                !q ? true : c.title.toLowerCase().includes(q)
              );
              if (conversations.length === 0) {
                return (
                  <div className="h-full flex items-center justify-center text-[11.5px] text-white/40">
                    {s.noConversations}
                  </div>
                );
              }
              if (filtered.length === 0) {
                return (
                  <div className="h-full flex items-center justify-center text-[11.5px] text-white/40">
                    {s.noHistoryMatches}
                  </div>
                );
              }

              const startOfToday = new Date();
              startOfToday.setHours(0, 0, 0, 0);
              const todayMs = startOfToday.getTime();
              const yesterdayMs = todayMs - 86_400_000;

              const groups: { label: string; items: ConversationSummary[] }[] = [
                { label: s.today, items: filtered.filter((c) => c.updated_at >= todayMs) },
                {
                  label: s.yesterday,
                  items: filtered.filter(
                    (c) => c.updated_at >= yesterdayMs && c.updated_at < todayMs
                  ),
                },
                { label: s.earlier, items: filtered.filter((c) => c.updated_at < yesterdayMs) },
              ].filter((g) => g.items.length > 0);

              return groups.map((group) => (
                <div key={group.label} className="space-y-1">
                  <div className="px-1 text-[10px] font-medium tracking-wider uppercase text-white/35">
                    {group.label}
                  </div>
                  {group.items.map((conv) => {
                    const isCurrent = conv.id === activeConversationId;
                    return (
                      <div
                        key={conv.id}
                        onClick={() => {
                          onSelectConversation(conv.id);
                          setShowHistory(false);
                        }}
                        className={`group relative rounded-xl px-2.5 py-2 border cursor-pointer flex items-center gap-2.5 transition ${
                          isCurrent
                            ? "bg-white/[0.09] border-sky-400/35 text-white"
                            : "bg-white/[0.03] hover:bg-white/[0.06] border-white/[0.07] text-white/80"
                        }`}
                      >
                        <div
                          className={`w-6 h-6 rounded-lg flex items-center justify-center shrink-0 ${
                            isCurrent
                              ? "bg-sky-500/20 text-sky-300"
                              : "bg-white/[0.05] text-white/40 group-hover:text-white/70"
                          }`}
                        >
                          <MessageSquare className="w-3 h-3" />
                        </div>

                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5">
                            {isCurrent && (
                              <span className="w-1.5 h-1.5 rounded-full bg-sky-400 shrink-0" />
                            )}
                            <span className="text-[11.5px] font-medium truncate">
                              {conv.title}
                            </span>
                          </div>
                          <div className="flex items-center gap-1.5 text-[10px] text-white/40 mt-0.5">
                            <span>
                              {conv.message_count} {s.messagesUnit}
                            </span>
                            <span>·</span>
                            <span>{formatRelativeTime(conv.updated_at, lang)}</span>
                          </div>
                        </div>

                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            onDeleteConversation(conv.id);
                          }}
                          title={s.deleteConv}
                          className="p-1.5 rounded-lg opacity-0 group-hover:opacity-100 text-white/40 hover:text-rose-300 hover:bg-rose-500/20 transition shrink-0"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    );
                  })}
                </div>
              ));
            })()}
          </div>
        </div>
      ) : (
        <>
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
              className="flex-1 overflow-y-auto space-y-2 pr-1 text-[12.5px] leading-relaxed select-text"
            >
              {messages.map((m) => (
                <div
                  key={m.id}
                  ref={(el) => {
                    if (m.role === "user") msgRefs.current[m.id] = el;
                  }}
                  className={`flex ${m.role === "user" ? "justify-end" : "justify-start"}`}
                >
                <div
                  className={`max-w-[90%] rounded-2xl px-3 py-2 ${
                    m.role === "user"
                      ? "rounded-tr-sm bg-sky-500/20 border border-sky-400/25 text-white"
                      : m.isError
                      ? "rounded-tl-sm bg-rose-500/15 border border-rose-400/30 text-rose-200"
                      : "rounded-tl-sm bg-white/[0.07] border border-white/10 text-white/90"
                  }`}
                >
                  {m.attachments && m.attachments.length > 0 && (
                    <div className="flex flex-wrap gap-1 mb-1.5">
                      {m.attachments.map((a, idx) => (
                        <span
                          key={idx}
                          title={a.file_path ?? a.label}
                          className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-black/35 text-[10px] text-sky-300 font-mono max-w-[185px]"
                        >
                          <AttachmentIcon kind={a.kind} />
                          <span className="truncate">{a.label}</span>
                        </span>
                      ))}
                    </div>
                  )}

                  {m.steps && m.steps.length > 0 && (
                    <div className="space-y-1.5 mb-1.5">
                      {m.steps.map((step) => (
                        <AgentStepCard
                          key={step.step_id}
                          step={step}
                          lang={lang}
                          onResolve={onResolveApproval}
                        />
                      ))}
                    </div>
                  )}

                  {m.role === "assistant" &&
                    ((isThinking &&
                      activeStreamId === m.id.toString() &&
                      thinkingLevel !== "off" &&
                      !m.text) ||
                      Boolean(m.thought)) && (
                      <ThoughtBox
                        thought={m.thought}
                        isLive={isThinking && activeStreamId === m.id.toString() && !m.text}
                        lang={lang}
                      />
                    )}

                  {m.role === "assistant" ? (
                    <div className="prose prose-invert prose-xs max-w-none [&_code]:text-sky-300 [&_code]:font-mono [&_code]:text-[11px] [&_p]:my-1 [&_ul]:pl-4 [&_ul]:list-disc [&_ol]:pl-4 [&_ol]:list-decimal">
                      {m.text ? (
                        <Markdown
                          components={{
                            pre: ({ children }) => (
                              <MarkdownCodePre copyLabel={s.copyCode} copiedLabel={s.copiedCode}>
                                {children}
                              </MarkdownCodePre>
                            ),
                          }}
                        >
                          {m.text}
                        </Markdown>
                      ) : m.steps && m.steps.length > 0 ? (
                        <span className="text-[11px] text-white/45">{s.workingTools}</span>
                      ) : isThinking &&
                        activeStreamId === m.id.toString() &&
                        thinkingLevel !== "off" ? null : (
                        <Markdown>...</Markdown>
                      )}
                      {m.isError && m.text.includes("Settings") && (
                        <button
                          onClick={onOpenSettings}
                          className="mt-1.5 px-2.5 py-1 rounded-lg bg-white/15 hover:bg-white/25 text-[11px] text-white font-medium transition"
                        >
                          {s.openSettings}
                        </button>
                      )}
                      {m.outputPaths && m.outputPaths.length > 0 && (
                        <div className="mt-1.5 flex flex-wrap gap-1.5">
                          {m.outputPaths.map((fp, idx) => (
                            <button
                              key={idx}
                              type="button"
                              onClick={() => void invoke("plugin_open_folder", { path: fp })}
                              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-emerald-500/15 hover:bg-emerald-500/30 border border-emerald-400/25 text-[10.5px] text-emerald-200 transition"
                            >
                              <FolderOpen className="w-3 h-3" />
                              <span>{sp.openFolder}</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="whitespace-pre-wrap">{m.text}</div>
                  )}
                </div>
              </div>
            ))}
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
            <div className="mt-2 space-y-1 shrink-0">
              <div className="flex flex-wrap gap-1.5">
                {attachments.map((att, idx) => (
                  <div
                    key={idx}
                    title={att.file_path ?? att.label}
                    className="flex items-center gap-1.5 px-2 py-1 rounded-lg bg-sky-500/15 border border-sky-400/25 text-[11px] text-sky-200 max-w-[180px]"
                  >
                    <AttachmentIcon kind={att.kind} />
                    <span className="truncate font-mono">{att.label}</span>
                    <button
                      onClick={() => onRemoveAttachment(idx)}
                      className="text-white/60 hover:text-white shrink-0"
                    >
                      <X className="w-3 h-3" />
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
                  <div className="flex flex-wrap gap-1">
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

            {installedPlugins.media_downloader &&
              enabledPlugins.media_downloader &&
              !slashMenuOpen &&
              (() => {
                const dlProg = pluginProgress.media_downloader;
                const isDlRunning = dlProg?.stage === "running";
                const urlMatch = /(https?:\/\/[^\s]+)/i.exec(chatInput);
                if (!isDlRunning && !urlMatch) return null;
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
                    <span className="text-[10px] text-emerald-300/90 flex items-center gap-1 mr-0.5">
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
                  </div>
                );
              })()}

            <form onSubmit={handleSubmit} className="flex items-center gap-2">
              <input
                ref={inputRef}
                type="text"
                value={chatInput}
                onChange={(e) => {
                  setSlashDismissed(false);
                  setSlashSelIdx(0);
                  onChangeInput(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (!slashMenuOpen || slashMatches.length === 0) return;
                  const safeIdx = Math.min(slashSelIdx, slashMatches.length - 1);
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setSlashSelIdx((safeIdx + 1) % slashMatches.length);
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setSlashSelIdx((safeIdx - 1 + slashMatches.length) % slashMatches.length);
                  } else if (e.key === "Tab") {
                    e.preventDefault();
                    applySlashSkill(slashMatches[safeIdx]);
                  } else if (e.key === "Enter") {
                    const picked = slashMatches[safeIdx];
                    if (chatInput.trim().toLowerCase() !== `/${picked.id.toLowerCase()}`) {
                      e.preventDefault();
                      applySlashSkill(picked);
                    }
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setSlashDismissed(true);
                  }
                }}
                onFocus={() => onFocusInput(true)}
                onBlur={() => onFocusInput(false)}
                placeholder={s.inputPlaceholder}
                className="flex-1 rounded-xl bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-3 py-1.5 text-[12.5px] text-white placeholder:text-white/40"
              />
              {isThinking ? (
                <button
                  type="button"
                  onClick={onStopStream}
                  className="px-3 py-1.5 rounded-xl bg-rose-500/80 hover:bg-rose-500 text-white font-semibold text-[12px] flex items-center gap-1 transition"
                >
                  <Square className="w-3 h-3 fill-current" />
                  <span>{s.stop}</span>
                </button>
              ) : (
                <button
                  type="submit"
                  className="px-3 py-1.5 rounded-xl bg-sky-500 hover:bg-sky-400 text-slate-950 font-semibold text-[12px] transition"
                >
                  {s.send}
                </button>
              )}
            </form>
          </div>
        </>
      )}
    </div>
  );
}
