import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Image as ImageIcon, Search, Sparkles, Star, Trash2 } from "lucide-react";
import { formatRelativeTime, getStrings } from "../i18n";
import type { MemoryItem, UiLang } from "../types";

interface MemoryTabProps {
  lang: UiLang;
  refreshTrigger: number;
  onAskAi: (item: MemoryItem) => void;
}

function MemoryThumb({ path }: { path: string }) {
  const [src, setSrc] = useState<string>("");

  useEffect(() => {
    let active = true;
    void invoke<string>("read_memory_image", { path })
      .then((url) => {
        if (active) setSrc(url);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [path]);

  if (!src) {
    return (
      <div className="w-12 h-10 rounded-lg bg-white/[0.06] flex items-center justify-center shrink-0">
        <ImageIcon className="w-4 h-4 text-white/30" />
      </div>
    );
  }

  return (
    <img
      src={src}
      alt="Memory thumbnail"
      className="w-12 h-10 rounded-lg object-cover border border-white/10 shrink-0 bg-black"
    />
  );
}

export default function MemoryTab({ lang, refreshTrigger, onAskAi }: MemoryTabProps) {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<MemoryItem[]>([]);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const s = getStrings(lang).memory;

  const loadMemories = (q: string) => {
    void invoke<MemoryItem[]>("list_memories", {
      query: q.trim() ? q : null,
      limit: 60,
    })
      .then(setItems)
      .catch(() => {});
  };

  useEffect(() => {
    const timer = window.setTimeout(() => loadMemories(query), 120);
    return () => window.clearTimeout(timer);
  }, [query, refreshTrigger]);

  const handleToggleStar = async (id: number) => {
    try {
      const starred = await invoke<boolean>("toggle_memory_star", { id });
      setItems((prev) =>
        prev.map((m) => (m.id === id ? { ...m, is_starred: starred } : m))
      );
    } catch {
      // ignore
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await invoke("delete_memory", { id });
      setItems((prev) => prev.filter((m) => m.id !== id));
    } catch {
      // ignore
    }
  };

  const handleCopy = async (id: number) => {
    try {
      await invoke("copy_memory_to_clipboard", { id });
      setCopiedId(id);
      window.setTimeout(() => setCopiedId(null), 1200);
    } catch {
      // ignore
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 space-y-2">
      <div className="relative shrink-0">
        <Search className="w-3.5 h-3.5 text-white/40 absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={s.searchPlaceholder}
          className="w-full rounded-xl bg-white/[0.07] border border-white/15 focus:border-sky-400/50 outline-none pl-8 pr-3 py-1.5 text-[12px] text-white placeholder:text-white/40"
        />
      </div>

      <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
        {items.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-center py-8 text-white/45 text-[12px]">
            <p>{query ? s.noMatches : s.empty}</p>
          </div>
        ) : (
          items.map((item) => {
            const isImg = item.kind === "screenshot" || item.kind === "clipboard_image";
            const badgeColor =
              item.kind === "screenshot"
                ? "bg-sky-500/20 text-sky-300"
                : item.kind === "clipboard_image"
                ? "bg-emerald-500/20 text-emerald-300"
                : "bg-purple-500/20 text-purple-300";
            const badgeLabel =
              item.kind === "screenshot"
                ? s.screenshot
                : item.kind === "clipboard_image"
                ? s.image
                : s.clipboard;

            return (
              <div
                key={item.id}
                className="rounded-xl bg-white/[0.05] hover:bg-white/[0.08] border border-white/10 p-2 flex items-center justify-between gap-2 transition"
              >
                <div className="flex items-center gap-2.5 min-w-0 flex-1">
                  {isImg && (item.thumb_path || item.file_path) && (
                    <MemoryThumb path={(item.thumb_path || item.file_path)!} />
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 text-[10.5px] text-white/50">
                      <span className={`px-1.5 py-0.5 rounded ${badgeColor}`}>
                        {badgeLabel}
                      </span>
                      <span className="truncate">{item.app_name}</span>
                      <span>·</span>
                      <span className="shrink-0">
                        {formatRelativeTime(item.updated_at, lang)}
                      </span>
                    </div>
                    <div className="text-[11.5px] text-white/85 mt-1 truncate font-mono">
                      {item.content_text || (isImg ? s.capturedImage : "")}
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-1 shrink-0">
                  <button
                    onClick={() => onAskAi(item)}
                    title={s.askAi}
                    className="px-2 py-1 rounded-lg bg-white/10 hover:bg-sky-500/30 text-[10.5px] text-white flex items-center gap-1 transition"
                  >
                    <Sparkles className="w-3 h-3 text-sky-300" />
                    <span>{s.ask}</span>
                  </button>
                  <button
                    onClick={() => void handleCopy(item.id)}
                    title={s.copyTip}
                    className="p-1.5 rounded-lg bg-white/[0.06] hover:bg-white/15 text-white/70 hover:text-white transition"
                  >
                    <Copy className={`w-3 h-3 ${copiedId === item.id ? "text-emerald-400" : ""}`} />
                  </button>
                  <button
                    onClick={() => void handleToggleStar(item.id)}
                    title={item.is_starred ? s.unstarTip : s.starTip}
                    className="p-1.5 rounded-lg bg-white/[0.06] hover:bg-white/15 transition"
                  >
                    <Star
                      className={`w-3 h-3 ${
                        item.is_starred ? "fill-amber-400 text-amber-400" : "text-white/45"
                      }`}
                    />
                  </button>
                  <button
                    onClick={() => void handleDelete(item.id)}
                    title={s.deleteTip}
                    className="p-1.5 rounded-lg bg-white/[0.06] hover:bg-rose-500/25 text-white/45 hover:text-rose-300 transition"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
