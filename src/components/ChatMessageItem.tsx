import React, { memo, useMemo, useState } from "react";
import Markdown from "react-markdown";
import { invoke } from "@tauri-apps/api/core";
import {
  Brain,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  FileText,
  Film,
  Folder,
  FolderOpen,
  Headphones,
  Loader2,
  Paperclip,
  Pencil,
  Play,
  RotateCw,
  ShieldAlert,
  X,
  XCircle,
} from "lucide-react";
import { getStrings } from "../i18n";
import type {
  AgentStep,
  ApprovalDecision,
  AttachmentPayload,
  ChatMessage,
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

export function AttachmentIcon({ kind }: { kind: AttachmentPayload["kind"] }) {
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

  const liveSnippet = useMemo(() => {
    if (!isLive || open || !thought) return "";
    const lines = thought
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter(Boolean);
    const tail = lines[lines.length - 1] ?? "";
    return tail.length > 42 ? `…${tail.slice(-42)}` : tail;
  }, [isLive, open, thought]);

  return (
    <div className="mb-1 min-w-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="max-w-full inline-flex items-center gap-1.5 py-0.5 text-[11px] text-white/45 hover:text-white/80 transition cursor-pointer select-none"
      >
        {isLive ? (
          <span className="w-1.5 h-1.5 rounded-full bg-white/75 animate-pulse shrink-0" />
        ) : (
          <Brain className="w-3 h-3 text-white/40 shrink-0" />
        )}
        <span className={`shrink-0 ${isLive ? "text-white/65 animate-pulse" : ""}`}>
          {isLive ? s.thinkingActive : s.thoughtSaved}
        </span>
        {liveSnippet && (
          <span className="truncate text-[10px] text-white/35 max-w-[170px] font-normal">
            {liveSnippet}
          </span>
        )}
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

export function MediaOutputCard({
  filePath,
  lang,
}: {
  filePath: string;
  lang: UiLang;
}) {
  const s = getStrings(lang).chat;
  const sp = getStrings(lang).plugins;
  const [audioSrc, setAudioSrc] = useState<string | null>(null);
  const [loadingAudio, setLoadingAudio] = useState(false);
  const [audioErr, setAudioErr] = useState<string | null>(null);

  const fileName = filePath.split(/[/\\]/).pop() || filePath;
  const ext = (fileName.split(".").pop() ?? "").toUpperCase();
  const isAudio = /^(MP3|FLAC|M4A|WAV|OGG|OPUS|AAC)$/i.test(ext);

  const handleToggleListen = async () => {
    if (audioSrc) {
      setAudioSrc(null);
      setAudioErr(null);
      return;
    }
    setLoadingAudio(true);
    setAudioErr(null);
    try {
      const dataUrl = await invoke<string>("read_local_audio_data_url", { path: filePath });
      setAudioSrc(dataUrl);
    } catch (err) {
      setAudioErr(String(err));
    } finally {
      setLoadingAudio(false);
    }
  };

  return (
    <div className="mt-1.5 rounded-xl bg-emerald-500/[0.08] border border-emerald-400/25 p-2 space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 min-w-0">
          {isAudio ? (
            <Headphones className="w-3.5 h-3.5 text-emerald-300 shrink-0" />
          ) : (
            <Film className="w-3.5 h-3.5 text-emerald-300 shrink-0" />
          )}
          <span className="text-[11px] font-medium text-white truncate" title={filePath}>
            {fileName}
          </span>
          {ext && (
            <span className="px-1.5 py-0.5 rounded bg-emerald-500/20 text-[9px] font-mono text-emerald-200 shrink-0">
              {ext}
            </span>
          )}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        {isAudio && (
          <button
            type="button"
            disabled={loadingAudio}
            onClick={() => void handleToggleListen()}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-emerald-500/20 hover:bg-emerald-500/35 border border-emerald-400/30 text-[10.5px] text-emerald-100 transition"
          >
            {loadingAudio ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <Play className="w-3 h-3 fill-current" />
            )}
            <span>{audioSrc ? s.mediaStopListen : s.mediaListen}</span>
          </button>
        )}
        <button
          type="button"
          onClick={() => void invoke("open_local_media_file", { path: filePath }).catch(() => {})}
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-white/[0.08] hover:bg-white/[0.16] border border-white/10 text-[10.5px] text-white/90 transition"
        >
          <Play className="w-2.5 h-2.5" />
          <span>{s.mediaOpenFile}</span>
        </button>
        <button
          type="button"
          onClick={() => void invoke("plugin_open_folder", { path: filePath }).catch(() => {})}
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-white/[0.08] hover:bg-white/[0.16] border border-white/10 text-[10.5px] text-white/80 transition"
        >
          <FolderOpen className="w-3 h-3" />
          <span>{sp.openFolder}</span>
        </button>
      </div>

      {audioErr && <div className="text-[10px] text-rose-300">{audioErr}</div>}
      {audioSrc && (
        <audio
          src={audioSrc}
          controls
          autoPlay
          className="w-full h-8 rounded-lg opacity-95"
        />
      )}
    </div>
  );
}

interface ChatMessageItemProps {
  message: ChatMessage;
  lang: UiLang;
  isLiveStream: boolean;
  thinkingLevel: ThinkingLevel;
  canRetry?: boolean;
  onRetry?: () => void;
  canEdit?: boolean;
  onEditMessage?: (newText: string) => void;
  onResolveApproval: (stepId: string, decision: ApprovalDecision) => void;
  onOpenSettings: () => void;
  onRegisterUserRef: (id: number, el: HTMLDivElement | null) => void;
}

/**
 * Memoized single chat bubble so typewriter/thought stream ticks only re-render the active message
 * instead of re-parsing Markdown across the entire conversation history.
 */
export const ChatMessageItem = memo(function ChatMessageItem({
  message: m,
  lang,
  isLiveStream,
  thinkingLevel,
  canRetry = false,
  onRetry,
  canEdit = false,
  onEditMessage,
  onResolveApproval,
  onOpenSettings,
  onRegisterUserRef,
}: ChatMessageItemProps) {
  const s = getStrings(lang).chat;
  const sp = getStrings(lang).plugins;
  const [copiedReply, setCopiedReply] = useState(false);
  const [isEditingUser, setIsEditingUser] = useState(false);
  const [editDraft, setEditDraft] = useState("");

  const markdownComponents = useMemo(
    () => ({
      pre: ({ children }: { children?: React.ReactNode }) => (
        <MarkdownCodePre copyLabel={s.copyCode} copiedLabel={s.copiedCode}>
          {children}
        </MarkdownCodePre>
      ),
      a: ({ href, children }: { href?: string; children?: React.ReactNode }) => {
        const cleanHref = (href ?? "").trim();
        const isBrowseable = /^(?:https?|file):\/\//i.test(cleanHref);
        return (
          <a
            href={isBrowseable ? cleanHref : "#"}
            title={isBrowseable ? `${s.openLinkTip}: ${cleanHref}` : undefined}
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              if (!isBrowseable) return;
              void invoke("open_url_in_builtin_browser", {
                url: cleanHref,
                watch: true,
              }).catch(() => {
                if (/^https?:\/\//i.test(cleanHref)) {
                  void invoke("open_external_url", { url: cleanHref }).catch(() => {});
                }
              });
            }}
            className="text-sky-300 hover:text-sky-200 underline decoration-sky-400/45 underline-offset-2 break-all cursor-pointer"
          >
            {children}
          </a>
        );
      },
    }),
    [s.copyCode, s.copiedCode, s.openLinkTip]
  );

  const showLiveThought = isLiveStream && thinkingLevel !== "off" && !m.text;

  const downloadedMediaPaths = useMemo(() => {
    const out: string[] = [];
    if (m.steps) {
      for (const step of m.steps) {
        if (step.tool_name === "download_media" && step.status === "done" && step.preview) {
          const match = /^Saved:\s*(.+)$/m.exec(step.preview);
          if (match?.[1]) {
            const p = match[1].trim();
            if (p && !out.includes(p)) out.push(p);
          }
        }
      }
    }
    return out;
  }, [m.steps]);

  return (
    <div
      ref={
        m.role === "user"
          ? (el) => {
              onRegisterUserRef(m.id, el);
            }
          : undefined
      }
      className={`flex ${m.role === "user" ? "justify-end" : "justify-start"}`}
    >
      <div
        className={`group/msg max-w-[90%] rounded-2xl px-3 py-2 ${
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

        {m.role === "assistant" && (showLiveThought || Boolean(m.thought)) && (
          <ThoughtBox thought={m.thought} isLive={showLiveThought} lang={lang} />
        )}

        {m.role === "assistant" ? (
          <div className="prose prose-invert prose-xs max-w-none overflow-x-auto [&_code]:text-sky-300 [&_code]:font-mono [&_code]:text-[11px] [&_p]:my-1 [&_ul]:pl-4 [&_ul]:list-disc [&_ol]:pl-4 [&_ol]:list-decimal [&_table]:w-full [&_table]:text-[11px] [&_table]:border-collapse [&_th]:border [&_th]:border-white/15 [&_th]:bg-white/[0.06] [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_td]:border [&_td]:border-white/10 [&_td]:px-2 [&_td]:py-1">
            {m.text ? (
              <Markdown components={markdownComponents}>{m.text}</Markdown>
            ) : m.steps && m.steps.length > 0 ? (
              <span className="text-[11px] text-white/45">{s.workingTools}</span>
            ) : isLiveStream && thinkingLevel !== "off" ? null : (
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
            {downloadedMediaPaths.map((fp) => (
              <MediaOutputCard key={fp} filePath={fp} lang={lang} />
            ))}
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
            {!isLiveStream && (m.text || m.isError) && (
              <div className="mt-1.5 pt-1 border-t border-white/[0.05] flex items-center justify-end gap-1.5 opacity-55 group-hover/msg:opacity-100 transition-opacity">
                {m.text && (
                  <button
                    type="button"
                    onClick={() => {
                      void navigator.clipboard.writeText(m.text).then(() => {
                        setCopiedReply(true);
                        window.setTimeout(() => setCopiedReply(false), 1400);
                      });
                    }}
                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-white/[0.05] hover:bg-white/[0.12] text-[10px] text-white/70 hover:text-white transition"
                  >
                    {copiedReply ? (
                      <Check className="w-2.5 h-2.5 text-emerald-400" />
                    ) : (
                      <Copy className="w-2.5 h-2.5" />
                    )}
                    <span>{copiedReply ? s.copiedReply : s.copyReply}</span>
                  </button>
                )}
                {canRetry && onRetry && (
                  <button
                    type="button"
                    onClick={onRetry}
                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-white/[0.05] hover:bg-sky-500/20 text-[10px] text-white/70 hover:text-sky-200 transition"
                  >
                    <RotateCw className="w-2.5 h-2.5" />
                    <span>{s.retryReply}</span>
                  </button>
                )}
              </div>
            )}
          </div>
        ) : isEditingUser ? (
          <div className="space-y-1.5 min-w-[220px]">
            <textarea
              autoFocus
              rows={2}
              value={editDraft}
              onChange={(e) => setEditDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  setIsEditingUser(false);
                } else if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  const clean = editDraft.trim();
                  if (clean && onEditMessage) {
                    setIsEditingUser(false);
                    onEditMessage(clean);
                  }
                }
              }}
              className="w-full rounded-lg bg-black/45 border border-sky-400/55 outline-none px-2 py-1 text-[12px] text-white resize-none leading-relaxed"
            />
            <div className="flex items-center justify-end gap-1.5">
              <button
                type="button"
                onClick={() => setIsEditingUser(false)}
                className="px-2 py-0.5 rounded-md bg-white/[0.08] hover:bg-white/[0.16] text-[10px] text-white/75 transition"
              >
                {s.editCancelBtn}
              </button>
              <button
                type="button"
                disabled={!editDraft.trim()}
                onClick={() => {
                  const clean = editDraft.trim();
                  if (!clean || !onEditMessage) return;
                  setIsEditingUser(false);
                  onEditMessage(clean);
                }}
                className="px-2 py-0.5 rounded-md bg-sky-500 hover:bg-sky-400 disabled:opacity-40 text-[10px] font-semibold text-slate-950 transition"
              >
                {s.editSaveBtn}
              </button>
            </div>
          </div>
        ) : (
          <div>
            <div className="whitespace-pre-wrap">{m.text}</div>
            {canEdit && onEditMessage && (
              <div className="mt-1 flex items-center justify-end opacity-0 group-hover/msg:opacity-100 transition-opacity">
                <button
                  type="button"
                  title={s.editMsgTip}
                  onClick={() => {
                    setEditDraft(m.text);
                    setIsEditingUser(true);
                  }}
                  className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-black/25 hover:bg-black/40 text-[9.5px] text-sky-100/80 hover:text-white transition"
                >
                  <Pencil className="w-2.5 h-2.5" />
                  <span>{s.editMsgTip}</span>
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
});
