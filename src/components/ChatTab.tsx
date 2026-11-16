import React, { useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import {
  Bot,
  Brain,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  History,
  Loader2,
  MessageSquare,
  Paperclip,
  Plus,
  Search,
  ShieldAlert,
  Square,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import { formatRelativeTime, getStrings } from "../i18n";
import type {
  AgentStep,
  ApprovalDecision,
  AttachmentPayload,
  ChatMessage,
  ConversationSummary,
  MemoryItem,
  ThinkingLevel,
  UiLang,
} from "../types";

interface ChatTabProps {
  lang: UiLang;
  agentMode: boolean;
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
    <div className="mb-1.5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-lg bg-white/[0.06] hover:bg-white/[0.12] border border-white/10 text-[11px] text-sky-200/85 transition cursor-pointer"
      >
        <Brain className={`w-3 h-3 shrink-0 ${isLive ? "animate-pulse text-sky-300" : "text-white/55"}`} />
        <span>{isLive ? s.thinkingActive : s.thoughtSaved}</span>
        {open ? (
          <ChevronDown className="w-3 h-3 text-white/50 shrink-0" />
        ) : (
          <ChevronRight className="w-3 h-3 text-white/50 shrink-0" />
        )}
      </button>
      {open && (
        <div className="mt-1.5 rounded-xl bg-black/45 border border-white/10 px-2.5 py-2 text-[11px] text-white/70 whitespace-pre-wrap break-words max-h-36 overflow-y-auto leading-relaxed">
          {thought?.trim() ? thought : "..."}
        </div>
      )}
    </div>
  );
}

export const THINKING_STEPS: ThinkingLevel[] = ["off", "low", "medium", "high", "max"];

export function ThinkingSlider({
  value,
  lang,
  disabled,
  compact,
  onChange,
}: {
  value: ThinkingLevel;
  lang: UiLang;
  disabled?: boolean;
  compact?: boolean;
  onChange: (next: ThinkingLevel) => void;
}) {
  const s = getStrings(lang).chat;
  const idx = Math.max(0, THINKING_STEPS.indexOf(value));
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const pickByClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.max(0, Math.min(0.999, (clientX - rect.left) / rect.width));
    const nextIdx = Math.floor(ratio * 5);
    const next = THINKING_STEPS[nextIdx];
    if (next && next !== value) {
      onChange(next);
    }
  };

  const accentBar =
    idx === 0
      ? "bg-white/25"
      : idx === 1
      ? "bg-white/60"
      : idx === 2
      ? "bg-sky-400"
      : idx === 3
      ? "bg-indigo-400"
      : "bg-amber-400 shadow-[0_0_8px_rgba(251,191,36,0.75)]";

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label={s.thinkPrefix}
      aria-valuemin={0}
      aria-valuemax={4}
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
      className={`relative grid grid-cols-5 items-center rounded-lg bg-[#08090d] border border-white/[0.09] p-[2px] select-none touch-none ${
        compact ? "w-[162px] h-[24px]" : "w-[196px] h-[26px]"
      } ${disabled ? "opacity-45 pointer-events-none" : "cursor-pointer"}`}
    >
      {/* Smooth Gliding Elevated Thumb (Codex / Claude Code matte glass) */}
      <div
        className="pointer-events-none absolute top-[2px] bottom-[2px] left-[2px] rounded-[6px] bg-white/[0.11] border border-white/[0.15] shadow-[0_1px_4px_rgba(0,0,0,0.65)] flex flex-col justify-end items-center pb-[1.5px] transition-transform duration-200 ease-out"
        style={{
          width: "calc((100% - 4px) / 5)",
          transform: `translateX(${idx * 100}%)`,
        }}
      >
        <span className={`w-3.5 h-[1.5px] rounded-full transition-colors ${accentBar}`} />
      </div>

      {THINKING_STEPS.map((step, i) => {
        const active = i === idx;
        const stepInfo = s.thinkLevels[step];
        return (
          <div
            key={step}
            title={stepInfo.desc}
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
}: ChatTabProps) {
  const [showHistory, setShowHistory] = useState(false);
  const [historySearch, setHistorySearch] = useState("");
  const [activeUserIdx, setActiveUserIdx] = useState(0);
  const chatBottomRef = useRef<HTMLDivElement | null>(null);
  const scrollBoxRef = useRef<HTMLDivElement | null>(null);
  const msgRefs = useRef<Record<number, HTMLDivElement | null>>({});
  const s = getStrings(lang).chat;

  const userMessages = messages.filter((m) => m.role === "user");

  useEffect(() => {
    if (!showHistory) {
      chatBottomRef.current?.scrollIntoView({ behavior: "smooth" });
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
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "start" });
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
          onClick={() => setShowHistory((v) => !v)}
          className={`flex items-center gap-1 px-1.5 py-0.5 rounded-lg text-[11px] min-w-0 transition ${
            showHistory
              ? "bg-white/15 text-white"
              : "text-white/55 hover:text-white hover:bg-white/[0.07]"
          }`}
        >
          <History className="w-3 h-3 shrink-0" />
          <span className="truncate max-w-[76px]">
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

          <ThinkingSlider
            compact
            value={thinkingLevel}
            lang={lang}
            disabled={isThinking}
            onChange={onChangeThinkingLevel}
          />

          <button
            onClick={() => {
              setShowHistory(false);
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

      {/* Redesigned Past Conversations Panel */}
      {showHistory ? (
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
                    <div className="prose prose-invert prose-xs max-w-none [&_pre]:bg-black/55 [&_pre]:p-2 [&_pre]:rounded-lg [&_pre]:overflow-x-auto [&_pre]:my-1.5 [&_code]:text-sky-300 [&_code]:font-mono [&_code]:text-[11px] [&_p]:my-1 [&_ul]:pl-4 [&_ul]:list-disc [&_ol]:pl-4 [&_ol]:list-decimal">
                      {m.text ? (
                        <Markdown>{m.text}</Markdown>
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
                    </div>
                  ) : (
                    <div className="whitespace-pre-wrap">{m.text}</div>
                  )}
                </div>
              </div>
            ))}
              <div ref={chatBottomRef} />
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
              </div>
            </div>
          )}

          {/* Pending Multi-Attachments Strip */}
          {attachments.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5 shrink-0">
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
          )}

          {/* Input Bar */}
          <form onSubmit={handleSubmit} className="mt-2 flex items-center gap-2 shrink-0">
            <input
              type="text"
              value={chatInput}
              onChange={(e) => onChangeInput(e.target.value)}
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
        </>
      )}
    </div>
  );
}
