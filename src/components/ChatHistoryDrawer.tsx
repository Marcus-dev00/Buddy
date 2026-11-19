import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Download, Monitor, Pencil, Search, Smartphone, Trash2, X } from "lucide-react";
import { formatRelativeTime, getStrings } from "../i18n";
import type { ConversationSummary, UiLang } from "../types";

export interface ChatHistoryDrawerProps {
  lang: UiLang;
  conversations: ConversationSummary[];
  activeConversationId: number | null;
  sidebar?: boolean;
  onSelectConversation: (id: number) => void;
  onDeleteConversation: (id: number) => void;
  onRenameConversation: (id: number, title: string) => void;
  onClose: () => void;
}

export function ChatHistoryDrawer({
  lang,
  conversations,
  activeConversationId,
  sidebar = false,
  onSelectConversation,
  onDeleteConversation,
  onRenameConversation,
  onClose,
}: ChatHistoryDrawerProps) {
  const [historySearch, setHistorySearch] = useState("");
  const [searchResults, setSearchResults] = useState<ConversationSummary[] | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [exportedId, setExportedId] = useState<number | null>(null);
  const s = getStrings(lang).chat;

  const handleExportMd = async (id: number) => {
    try {
      const saved = await invoke<string | null>("export_conversation_markdown", {
        conversationId: id,
      });
      if (saved) {
        setExportedId(id);
        window.setTimeout(() => setExportedId((cur) => (cur === id ? null : cur)), 1600);
      }
    } catch {
      // Ignore cancelled or failed export
    }
  };

  useEffect(() => {
    const q = historySearch.trim();
    if (!q) {
      setSearchResults(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void invoke<ConversationSummary[]>("search_conversations", { query: q })
        .then((res) => {
          if (!cancelled) setSearchResults(res);
        })
        .catch(() => {
          if (!cancelled) setSearchResults(null);
        });
    }, 160);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [historySearch]);

  const q = historySearch.trim().toLowerCase();
  const baseList = searchResults ?? conversations;
  const nonEmpty = baseList.filter((c) => c.message_count > 0);
  const filtered = searchResults
    ? nonEmpty
    : nonEmpty.filter((c) => (!q ? true : c.title.toLowerCase().includes(q)));

  const commitRename = (id: number) => {
    const clean = editTitle.trim();
    setEditingId(null);
    if (!clean) return;
    onRenameConversation(id, clean);
  };

  const renderBody = () => {
    if (conversations.filter((c) => c.message_count > 0).length === 0) {
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
        items: filtered.filter((c) => c.updated_at >= yesterdayMs && c.updated_at < todayMs),
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
          const isEditing = editingId === conv.id;
          return (
            <div
              key={conv.id}
              onClick={() => {
                if (isEditing) return;
                onSelectConversation(conv.id);
                if (!sidebar) onClose();
              }}
              className={`group relative rounded-xl px-2.5 py-2 border cursor-pointer flex items-center gap-2.5 transition ${
                isCurrent
                  ? "bg-white/[0.11] border-white/25 text-white"
                  : "bg-white/[0.03] hover:bg-white/[0.06] border-white/[0.07] text-white/80"
              }`}
            >
              <div
                title={
                  conv.source === "mobile"
                    ? lang === "zh"
                      ? "手机版对话"
                      : "Mobile Chat"
                    : lang === "zh"
                    ? "电脑版对话"
                    : "Desktop Chat"
                }
                className={`w-6 h-6 rounded-lg flex items-center justify-center shrink-0 ${
                  conv.source === "mobile"
                    ? isCurrent
                      ? "bg-emerald-500/22 text-emerald-300"
                      : "bg-emerald-500/12 text-emerald-300/80"
                    : isCurrent
                    ? "bg-white/15 text-white"
                    : "bg-white/[0.05] text-white/45 group-hover:text-white/75"
                }`}
              >
                {conv.source === "mobile" ? (
                  <Smartphone className="w-3 h-3" />
                ) : (
                  <Monitor className="w-3 h-3" />
                )}
              </div>

              <div className="min-w-0 flex-1">
                {isEditing ? (
                  <div
                    className="flex items-center gap-1"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <input
                      type="text"
                      autoFocus
                      value={editTitle}
                      onChange={(e) => setEditTitle(e.target.value)}
                      onBlur={() => commitRename(conv.id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          commitRename(conv.id);
                        } else if (e.key === "Escape") {
                          setEditingId(null);
                        }
                      }}
                      className="flex-1 min-w-0 rounded-md bg-black/45 border border-sky-400/55 outline-none px-1.5 py-0.5 text-[11px] text-white"
                    />
                    <button
                      type="button"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        commitRename(conv.id);
                      }}
                      className="p-1 rounded bg-sky-500/25 text-sky-200 hover:bg-sky-500/40"
                    >
                      <Check className="w-2.5 h-2.5" />
                    </button>
                  </div>
                ) : (
                  <div
                    className="flex items-center gap-1.5"
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      setEditingId(conv.id);
                      setEditTitle(conv.title);
                    }}
                  >
                    {isCurrent && <span className="w-1.5 h-1.5 rounded-full bg-sky-400 shrink-0" />}
                    <span className="text-[11.5px] font-medium truncate">{conv.title}</span>
                  </div>
                )}
                <div className="flex items-center gap-1.5 text-[10px] text-white/40 mt-0.5">
                  <span>
                    {conv.message_count} {s.messagesUnit}
                  </span>
                  <span>·</span>
                  <span>{formatRelativeTime(conv.updated_at, lang)}</span>
                </div>
              </div>

              {!isEditing && (
                <div className="flex items-center gap-0.5 shrink-0">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleExportMd(conv.id);
                    }}
                    title={exportedId === conv.id ? s.exportMdDone : s.exportMd}
                    className={`p-1.5 rounded-lg transition ${
                      exportedId === conv.id
                        ? "opacity-100 text-emerald-300 bg-emerald-500/20"
                        : "opacity-0 group-hover:opacity-100 text-white/40 hover:text-emerald-200 hover:bg-white/10"
                    }`}
                  >
                    {exportedId === conv.id ? (
                      <Check className="w-3 h-3" />
                    ) : (
                      <Download className="w-3 h-3" />
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      setEditingId(conv.id);
                      setEditTitle(conv.title);
                    }}
                    title={s.renameConv}
                    className="p-1.5 rounded-lg opacity-0 group-hover:opacity-100 text-white/40 hover:text-sky-200 hover:bg-white/10 transition"
                  >
                    <Pencil className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onDeleteConversation(conv.id);
                    }}
                    title={s.deleteConv}
                    className="p-1.5 rounded-lg opacity-0 group-hover:opacity-100 text-white/40 hover:text-rose-300 hover:bg-rose-500/20 transition"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    ));
  };

  return (
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

      <div className="flex-1 overflow-y-auto space-y-2.5 pr-1">{renderBody()}</div>
    </div>
  );
}
