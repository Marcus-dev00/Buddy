import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Bell,
  Check,
  Copy,
  FolderOpen,
  Image as ImageIcon,
  Maximize2,
  Pin,
  Plus,
  ScanText,
  Search,
  Sparkles,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { formatRelativeTime, getStrings } from "../i18n";
import type { ClipboardParseResult, MemoryItem, OcrResult, ReminderItem, UiLang } from "../types";

type MemoryFilter = "all" | "images" | "clipboard" | "reminders";

function BuddyMiniLogo() {
  return (
    <svg width="16" height="10" viewBox="0 0 16 10" className="shrink-0">
      <rect
        x="0.5"
        y="0.5"
        width="15"
        height="9"
        rx="4.5"
        fill="#0b0b0f"
        stroke="rgba(255,255,255,0.28)"
        strokeWidth="1"
      />
      <rect x="4.5" y="2.8" width="2.3" height="4.4" rx="1.15" fill="#ffffff" />
      <rect x="9.2" y="2.8" width="2.3" height="4.4" rx="1.15" fill="#ffffff" />
    </svg>
  );
}

interface MemoryTabProps {
  lang: UiLang;
  isWindowMode?: boolean;
  refreshTrigger: number;
  enabledPlugins?: Record<string, boolean>;
  onAskAi: (item: MemoryItem) => void;
  onOcrExtracted?: (text: string) => void;
}

function MemoryImageView({
  path,
  className,
}: {
  path: string;
  className: string;
}) {
  const [src, setSrc] = useState<string>("");

  useEffect(() => {
    let active = true;
    setSrc("");
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
      <div className={`${className} bg-white/[0.04] flex items-center justify-center`}>
        <ImageIcon className="w-5 h-5 text-white/25" />
      </div>
    );
  }

  return <img src={src} alt="Captured memory" className={className} />;
}

export default function MemoryTab({
  lang,
  isWindowMode = false,
  refreshTrigger,
  enabledPlugins = {},
  onAskAi,
  onOcrExtracted,
}: MemoryTabProps) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<MemoryFilter>("all");
  const [starredOnly, setStarredOnly] = useState(false);
  const [items, setItems] = useState<MemoryItem[]>([]);
  const [reminders, setReminders] = useState<ReminderItem[]>([]);
  const [remTitle, setRemTitle] = useState("");
  const [remTimeSpec, setRemTimeSpec] = useState("20m");
  const [remRepeat, setRemRepeat] = useState<"none" | "daily" | "weekdays" | "weekly">("none");
  const [remError, setRemError] = useState<string | null>(null);
  const [addingRem, setAddingRem] = useState(false);
  const [selectedItem, setSelectedItem] = useState<MemoryItem | null>(null);
  const [selectedParsed, setSelectedParsed] = useState<ClipboardParseResult | null>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [ocrDoneId, setOcrDoneId] = useState<number | null>(null);
  const s = getStrings(lang).memory;
  const sp = getStrings(lang).plugins;

  const loadMemories = (q: string) => {
    void invoke<MemoryItem[]>("list_memories", {
      query: q.trim() ? q : null,
      limit: 80,
    })
      .then(setItems)
      .catch(() => {});
  };

  const loadReminders = () => {
    void invoke<ReminderItem[]>("list_reminders", { includeCompleted: true })
      .then(setReminders)
      .catch(() => {});
  };

  useEffect(() => {
    const timer = window.setTimeout(() => loadMemories(query), 120);
    return () => window.clearTimeout(timer);
  }, [query, refreshTrigger]);

  useEffect(() => {
    loadReminders();
  }, [refreshTrigger]);

  useEffect(() => {
    const u1 = listen("buddy://reminders-changed", () => loadReminders());
    const u2 = listen("buddy://reminder-due", () => loadReminders());
    return () => {
      void u1.then((fn) => fn());
      void u2.then((fn) => fn());
    };
  }, []);

  const handleAddReminder = async (e: React.FormEvent) => {
    e.preventDefault();
    const title = remTitle.trim();
    const spec = remTimeSpec.trim();
    if (!title || !spec || addingRem) return;
    setRemError(null);
    setAddingRem(true);
    try {
      const relMatch = /^(\d+(?:\.\d+)?)\s*(m|min|mins|h|hr|hrs|分钟|小时)?$/i.exec(spec);
      let delayMinutes: number | null = null;
      let atTime: string | null = null;
      if (relMatch && !spec.includes(":")) {
        const val = parseFloat(relMatch[1]);
        const unit = (relMatch[2] ?? "m").toLowerCase();
        delayMinutes = unit.startsWith("h") || unit.includes("小时") ? val * 60 : val;
      } else {
        atTime = spec;
      }
      const created = await invoke<ReminderItem>("create_reminder", {
        title,
        delayMinutes,
        atTime,
        repeatRule: remRepeat === "none" ? null : remRepeat,
      });
      setReminders((prev) => [created, ...prev]);
      setRemTitle("");
    } catch (err) {
      setRemError(String(err));
    } finally {
      setAddingRem(false);
    }
  };

  useEffect(() => {
    setSelectedParsed(null);
    if (
      selectedItem &&
      selectedItem.kind === "clipboard_text" &&
      selectedItem.content_text &&
      enabledPlugins.clipboard_parser !== false
    ) {
      void invoke<ClipboardParseResult>("plugin_parse_clipboard", {
        text: selectedItem.content_text,
      })
        .then(setSelectedParsed)
        .catch(() => {});
    }
  }, [selectedItem, enabledPlugins.clipboard_parser]);

  const handleToggleStar = async (id: number) => {
    try {
      const starred = await invoke<boolean>("toggle_memory_star", { id });
      setItems((prev) =>
        prev.map((m) => (m.id === id ? { ...m, is_starred: starred } : m))
      );
      setSelectedItem((prev) => (prev && prev.id === id ? { ...prev, is_starred: starred } : prev));
    } catch {
      // ignore
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await invoke("delete_memory", { id });
      setItems((prev) => prev.filter((m) => m.id !== id));
      setSelectedItem((prev) => (prev && prev.id === id ? null : prev));
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

  const handleOcr = (item: MemoryItem) => {
    if (!item.file_path) return;
    void invoke<OcrResult>("plugin_offline_ocr", { source: item.file_path })
      .then((res) => {
        const extracted = res.text?.trim() || "";
        if (extracted) {
          void navigator.clipboard?.writeText(extracted).catch(() => {});
        }
        if (onOcrExtracted) {
          onOcrExtracted(
            extracted || (lang === "zh" ? "（未识别到文字）" : "(no text detected)")
          );
        }
        setSelectedItem(null);
        setOcrDoneId(item.id);
        window.setTimeout(() => setOcrDoneId(null), 1200);
      })
      .catch((err) => {
        if (onOcrExtracted) {
          onOcrExtracted(String(err));
          setSelectedItem(null);
        }
      });
  };

  const baseItems = (starredOnly ? items.filter((m) => m.is_starred) : items).filter(
    (m) => m.kind !== "fact"
  );
  const imageItems = baseItems.filter(
    (m) => m.kind === "screenshot" || m.kind === "clipboard_image"
  );
  const textItems = baseItems.filter((m) => m.kind === "clipboard_text");
  const filteredReminders = query.trim()
    ? reminders.filter((r) => r.title.toLowerCase().includes(query.trim().toLowerCase()))
    : reminders;
  const activeReminderCount = reminders.filter((r) => r.enabled).length;

  const showImages = filter === "all" || filter === "images";
  const showTexts = filter === "all" || filter === "clipboard";
  const isEmpty =
    (filter === "all" && imageItems.length === 0 && textItems.length === 0) ||
    (filter === "images" && imageItems.length === 0) ||
    (filter === "clipboard" && textItems.length === 0);

  return (
    <div
      className={`relative flex-1 flex flex-col min-h-0 ${
        isWindowMode ? "max-w-5xl w-full mx-auto space-y-4" : "space-y-2"
      }`}
    >
      {/* Toolbar: Horizontal single row in Window Mode, stacked compact in Island Mode */}
      <div
        className={
          isWindowMode
            ? "flex items-center justify-between gap-4 shrink-0"
            : "space-y-2 shrink-0"
        }
      >
        {!isWindowMode && (
          <div className="relative shrink-0">
            <Search className="w-3.5 h-3.5 text-white/40 absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={s.searchPlaceholder}
              className="w-full rounded-full bg-white/[0.06] border border-white/15 focus:border-white/35 outline-none pl-8 pr-3 py-1.5 text-[12px] text-white placeholder:text-white/40 transition"
            />
          </div>
        )}

        <div className="flex items-center justify-between gap-2 shrink-0">
          <div className="flex items-center gap-0.5 rounded-full bg-white/[0.045] p-0.5 border border-white/10 min-w-0 overflow-x-auto [&::-webkit-scrollbar]:hidden">
            {(
              [
                { id: "all", label: `${s.tabAll} (${baseItems.length})` },
                { id: "images", label: `${s.tabImages} (${imageItems.length})` },
                { id: "clipboard", label: `${s.tabClipboard} (${textItems.length})` },
                { id: "reminders", label: `${s.tabReminders} (${activeReminderCount})` },
              ] as const
            ).map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setFilter(tab.id)}
                className={`shrink-0 whitespace-nowrap rounded-full border font-medium transition-all duration-200 ${
                  isWindowMode ? "px-3.5 py-1 text-[12.5px]" : "px-2 py-0.5 text-[10.5px]"
                } ${
                  filter === tab.id
                    ? "bg-white text-black border-white"
                    : "border-transparent text-white/55 hover:text-white/90 hover:bg-white/[0.05]"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>

          {filter !== "reminders" && (
            <button
              type="button"
              onClick={() => setStarredOnly((v) => !v)}
              title={s.starredFilter}
              className={`rounded-full border shrink-0 transition ${
                isWindowMode ? "p-2" : "p-1.5"
              } ${
                starredOnly
                  ? "bg-amber-500/20 border-amber-400/35 text-amber-300"
                  : "bg-white/[0.05] border-white/10 text-white/45 hover:text-white"
              }`}
            >
              <Star className={`w-3.5 h-3.5 ${starredOnly ? "fill-amber-300" : ""}`} />
            </button>
          )}
        </div>

        {isWindowMode && (
          <div className="relative w-72 shrink-0">
            <Search className="w-4 h-4 text-white/40 absolute left-3.5 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={s.searchPlaceholder}
              className="w-full rounded-full bg-white/[0.06] border border-white/15 focus:border-white/35 outline-none pl-9 pr-3.5 py-1.5 text-[13px] text-white placeholder:text-white/40 transition"
            />
          </div>
        )}
      </div>

      {/* Scrollable Content */}
      <div className={`flex-1 overflow-y-auto pr-1 ${isWindowMode ? "space-y-5 pb-4" : "space-y-3"}`}>
        {filter === "reminders" ? (
          <div className={isWindowMode ? "max-w-2xl mx-auto w-full space-y-3" : "space-y-2.5"}>
            <div className="rounded-2xl bg-sky-500/[0.07] border border-sky-400/20 p-2.5 space-y-2">
              <div className="flex items-center gap-1.5 text-[11px] font-medium text-sky-200">
                <Bell className="w-3.5 h-3.5 shrink-0" />
                <span>{s.remindersTitle}</span>
              </div>
              <p className="text-[10.5px] text-white/55 leading-relaxed">{s.remindersHint}</p>
              <form onSubmit={(e) => void handleAddReminder(e)} className="space-y-1.5">
                <input
                  type="text"
                  value={remTitle}
                  onChange={(e) => setRemTitle(e.target.value)}
                  placeholder={s.remindersTitlePlaceholder}
                  className="w-full rounded-xl bg-black/35 border border-white/12 focus:border-sky-400/50 outline-none px-2.5 py-1.5 text-[11px] text-white placeholder:text-white/35"
                />
                <div className="flex items-center gap-1.5">
                  <input
                    type="text"
                    value={remTimeSpec}
                    onChange={(e) => setRemTimeSpec(e.target.value)}
                    placeholder={s.remindersTimePlaceholder}
                    className="w-28 rounded-xl bg-black/35 border border-white/12 focus:border-sky-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/35"
                  />
                  <select
                    value={remRepeat}
                    onChange={(e) =>
                      setRemRepeat(e.target.value as "none" | "daily" | "weekdays" | "weekly")
                    }
                    className="flex-1 rounded-xl bg-black/45 border border-white/12 outline-none px-2 py-1 text-[10.5px] text-white"
                  >
                    <option value="none">{s.repeatOnce}</option>
                    <option value="daily">{s.repeatDaily}</option>
                    <option value="weekdays">{s.repeatWeekdays}</option>
                    <option value="weekly">{s.repeatWeekly}</option>
                  </select>
                  <button
                    type="submit"
                    disabled={!remTitle.trim() || !remTimeSpec.trim() || addingRem}
                    className="px-2.5 py-1 rounded-xl bg-sky-500 hover:bg-sky-400 disabled:opacity-40 text-slate-950 text-[10.5px] font-semibold inline-flex items-center gap-1 shrink-0 transition"
                  >
                    <Plus className="w-3 h-3" />
                    <span>{s.remindersAddBtn}</span>
                  </button>
                </div>
                {remError && <div className="text-[10px] text-rose-300">{remError}</div>}
              </form>
            </div>

            {filteredReminders.length === 0 ? (
              <div className="rounded-2xl bg-white/[0.03] border border-white/10 p-5 text-center text-[11.5px] text-white/45">
                {query.trim() ? s.noMatches : s.remindersEmpty}
              </div>
            ) : (
              <div className="space-y-1.5">
                {filteredReminders.map((rem) => {
                  const dt = new Date(rem.fire_at_ms);
                  const whenLabel = `${dt.getMonth() + 1}/${dt.getDate()} ${String(
                    dt.getHours()
                  ).padStart(2, "0")}:${String(dt.getMinutes()).padStart(2, "0")}`;
                  const repLabel =
                    rem.repeat_rule === "daily"
                      ? s.repeatDaily
                      : rem.repeat_rule === "weekdays"
                      ? s.repeatWeekdays
                      : rem.repeat_rule === "weekly"
                      ? s.repeatWeekly
                      : s.repeatOnce;
                  return (
                    <div
                      key={rem.id}
                      className={`group rounded-xl border px-3 py-2 flex items-center justify-between gap-2 transition ${
                        rem.enabled
                          ? "bg-white/[0.045] hover:bg-white/[0.07] border-white/10"
                          : "bg-white/[0.02] border-white/[0.06] opacity-55"
                      }`}
                    >
                      <div className="min-w-0 flex-1 space-y-0.5">
                        <div className="text-[11.5px] text-white/90 font-medium truncate">
                          {rem.title}
                        </div>
                        <div className="flex items-center gap-1.5 text-[9.5px] text-white/45 font-mono">
                          <span className="text-sky-300/85">{whenLabel}</span>
                          <span>·</span>
                          <span className="px-1.5 py-[1px] rounded bg-white/[0.06] text-white/65">
                            {repLabel}
                          </span>
                        </div>
                      </div>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          role="switch"
                          aria-checked={rem.enabled}
                          onClick={() => {
                            void invoke<ReminderItem>("toggle_reminder", {
                              id: rem.id,
                              enabled: !rem.enabled,
                            })
                              .then((upd) =>
                                setReminders((prev) =>
                                  prev.map((x) => (x.id === upd.id ? upd : x))
                                )
                              )
                              .catch(() => {});
                          }}
                          className={`relative w-7 h-4 rounded-full transition-colors ${
                            rem.enabled ? "bg-sky-500" : "bg-white/20"
                          }`}
                        >
                          <span
                            className={`block w-3 h-3 rounded-full bg-white shadow transition-transform ${
                              rem.enabled ? "translate-x-[14px]" : "translate-x-[2px]"
                            }`}
                          />
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            void invoke("delete_reminder", { id: rem.id })
                              .then(() =>
                                setReminders((prev) => prev.filter((x) => x.id !== rem.id))
                              )
                              .catch(() => {});
                          }}
                          title={s.deleteTip}
                          className="p-1.5 rounded-lg opacity-60 group-hover:opacity-100 text-white/45 hover:text-rose-300 hover:bg-rose-500/20 transition"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        ) : isEmpty ? (
          <div className="h-full flex flex-col items-center justify-center text-center py-8 text-white/45 text-[12px]">
            <p>{query ? s.noMatches : s.empty}</p>
          </div>
        ) : (
          <>
            {/* SECTION 1: Screenshots & Images Grid */}
            {showImages && imageItems.length > 0 && (
              <div className="space-y-1.5">
                {filter === "all" && (
                  <div className="text-[10px] font-medium uppercase tracking-wider text-white/45 px-0.5">
                    {s.sectionImages} · {imageItems.length}
                  </div>
                )}
                <div
                  className={
                    isWindowMode
                      ? "grid grid-cols-3 lg:grid-cols-4 gap-3.5"
                      : "grid grid-cols-2 gap-2"
                  }
                >
                  {imageItems.map((item) => {
                    const imgPath = item.thumb_path || item.file_path || "";
                    return (
                      <div
                        key={item.id}
                        className="group rounded-2xl bg-white/[0.05] hover:bg-white/[0.08] border border-white/10 overflow-hidden flex flex-col transition"
                      >
                        <button
                          type="button"
                          onClick={() => setSelectedItem(item)}
                          className={`relative w-full bg-black/70 overflow-hidden cursor-zoom-in block text-left ${
                            isWindowMode ? "aspect-[16/10]" : "h-24"
                          }`}
                        >
                          <MemoryImageView
                            path={imgPath}
                            className="w-full h-full object-cover"
                          />
                          <div className="absolute inset-0 bg-black/0 group-hover:bg-black/25 transition flex items-center justify-center">
                            <Maximize2 className="w-4 h-4 text-white opacity-0 group-hover:opacity-90 transition" />
                          </div>
                          {item.is_starred && (
                            <Star className="w-3 h-3 fill-amber-400 text-amber-400 absolute top-1.5 right-1.5 drop-shadow" />
                          )}
                        </button>

                        <div className="p-2 space-y-1.5">
                          <div className="flex items-center justify-between gap-1 text-[10px] text-white/55">
                            <span className="truncate">{item.app_name}</span>
                            <span className="shrink-0">
                              {formatRelativeTime(item.updated_at, lang)}
                            </span>
                          </div>
                          <div className="flex items-center justify-between gap-1">
                            <button
                              type="button"
                              onClick={() => onAskAi(item)}
                              title={s.askAi}
                              className="px-2 py-0.5 rounded-lg bg-white/[0.11] hover:bg-white/[0.20] border border-white/10 text-[10px] font-medium text-white flex items-center gap-1 transition"
                            >
                              <BuddyMiniLogo />
                              <span>{s.ask}</span>
                            </button>

                            <div className="flex items-center gap-0.5">
                              {item.file_path && enabledPlugins.offline_ocr !== false && (
                                <button
                                  type="button"
                                  onClick={() => handleOcr(item)}
                                  title={sp.ocrAction}
                                  className="p-1 rounded-md hover:bg-white/15 text-white/60 hover:text-sky-300 transition"
                                >
                                  {ocrDoneId === item.id ? (
                                    <Check className="w-3 h-3 text-emerald-300" />
                                  ) : (
                                    <ScanText className="w-3 h-3" />
                                  )}
                                </button>
                              )}
                              {item.file_path && enabledPlugins.pin_to_screen !== false && (
                                <button
                                  type="button"
                                  onClick={() => {
                                    void invoke("plugin_pin_image", {
                                      source: item.file_path,
                                    }).catch(() => {});
                                  }}
                                  title={sp.pinAction}
                                  className="p-1 rounded-md hover:bg-white/15 text-white/55 hover:text-amber-300 transition"
                                >
                                  <Pin className="w-3 h-3" />
                                </button>
                              )}
                              <button
                                type="button"
                                onClick={() => void handleCopy(item.id)}
                                title={s.copyTip}
                                className="p-1 rounded-md hover:bg-white/15 text-white/55 hover:text-white transition"
                              >
                                {copiedId === item.id ? (
                                  <Check className="w-3 h-3 text-white" />
                                ) : (
                                  <Copy className="w-3 h-3" />
                                )}
                              </button>
                              <button
                                type="button"
                                onClick={() => void handleToggleStar(item.id)}
                                title={item.is_starred ? s.unstarTip : s.starTip}
                                className="p-1 rounded-md hover:bg-white/15 transition"
                              >
                                <Star
                                  className={`w-3 h-3 ${
                                    item.is_starred
                                      ? "fill-amber-400 text-amber-400"
                                      : "text-white/45"
                                  }`}
                                />
                              </button>
                              <button
                                type="button"
                                onClick={() => void handleDelete(item.id)}
                                title={s.deleteTip}
                                className="p-1 rounded-md hover:bg-rose-500/25 text-white/45 hover:text-rose-300 transition"
                              >
                                <Trash2 className="w-3 h-3" />
                              </button>
                            </div>
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* SECTION 2: Copied Clipboard Text */}
            {showTexts && textItems.length > 0 && (
              <div className="space-y-1.5">
                {filter === "all" && (
                  <div className="text-[10.5px] font-medium uppercase tracking-wider text-white/45 px-0.5">
                    {s.sectionClipboard} · {textItems.length}
                  </div>
                )}
                <div className={isWindowMode ? "grid grid-cols-1 lg:grid-cols-2 gap-3" : "space-y-1.5"}>
                {textItems.map((item) => (
                  <div
                    key={item.id}
                    className={`rounded-2xl bg-white/[0.05] hover:bg-white/[0.08] border border-white/10 space-y-1.5 transition ${
                      isWindowMode ? "p-3.5" : "p-2.5"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <button
                        type="button"
                        onClick={() => setSelectedItem(item)}
                        className="flex items-center gap-1.5 min-w-0 text-[10.5px] text-white/55 hover:text-white/85 transition text-left"
                      >
                        <span
                          className={`px-1.5 py-0.5 rounded shrink-0 ${
                            item.kind === "fact"
                              ? "bg-amber-500/20 text-amber-200"
                              : "bg-white/10 text-white/80"
                          }`}
                        >
                          {item.kind === "fact" ? "Fact" : s.clipboard}
                        </span>
                        <span className="truncate">{item.app_name}</span>
                        <span>·</span>
                        <span className="shrink-0">
                          {formatRelativeTime(item.updated_at, lang)}
                        </span>
                      </button>

                      <div className="flex items-center gap-1 shrink-0">
                        <button
                          type="button"
                          onClick={() => onAskAi(item)}
                          title={s.askAi}
                          className="px-2 py-0.5 rounded-lg bg-white/[0.11] hover:bg-white/[0.20] border border-white/10 text-[10px] font-medium text-white flex items-center gap-1.5 transition"
                        >
                          <BuddyMiniLogo />
                          <span>{s.ask}</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => void handleCopy(item.id)}
                          title={s.copyTip}
                          className="p-1 rounded-md bg-white/[0.05] hover:bg-white/15 text-white/65 hover:text-white transition"
                        >
                          {copiedId === item.id ? (
                            <Check className="w-3 h-3 text-white" />
                          ) : (
                            <Copy className="w-3 h-3" />
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={() => void handleToggleStar(item.id)}
                          title={item.is_starred ? s.unstarTip : s.starTip}
                          className="p-1 rounded-md bg-white/[0.05] hover:bg-white/15 transition"
                        >
                          <Star
                            className={`w-3 h-3 ${
                              item.is_starred ? "fill-amber-400 text-amber-400" : "text-white/45"
                            }`}
                          />
                        </button>
                        <button
                          type="button"
                          onClick={() => void handleDelete(item.id)}
                          title={s.deleteTip}
                          className="p-1 rounded-md bg-white/[0.05] hover:bg-rose-500/25 text-white/45 hover:text-rose-300 transition"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </div>

                    <button
                      type="button"
                      onClick={() => setSelectedItem(item)}
                      className={`w-full text-left text-white/90 hover:text-white font-mono line-clamp-2 break-all leading-relaxed cursor-pointer ${
                        isWindowMode ? "text-[12.5px]" : "text-[11.5px]"
                      }`}
                    >
                      {item.content_text || ""}
                    </button>
                  </div>
                ))}
                </div>
              </div>
            )}
          </>
        )}
      </div>

      {/* IN-PANEL / CENTERED LIGHTBOX MODAL */}
      {selectedItem && (
        <div
          onClick={() => setSelectedItem(null)}
          className={
            isWindowMode
              ? "fixed inset-0 z-50 bg-black/80 backdrop-blur-md flex items-center justify-center p-8"
              : "absolute inset-0 z-30 rounded-2xl bg-[#07070a]/95 backdrop-blur-md border border-white/15 p-3 flex flex-col space-y-2.5"
          }
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className={
              isWindowMode
                ? "w-full max-w-4xl max-h-[84vh] rounded-2xl bg-[#0b0b0f] border border-white/15 shadow-2xl p-4 flex flex-col min-h-0 space-y-3"
                : "flex-1 flex flex-col min-h-0 space-y-2.5"
            }
          >
            {/* Modal Header */}
            <div className="flex items-center justify-between gap-2 shrink-0">
              <div className="min-w-0 text-[11px] text-white/60 truncate">
                <span className="text-white font-medium">
                  {selectedItem.kind === "fact"
                    ? "Fact"
                    : selectedItem.kind === "clipboard_text"
                    ? s.clipboard
                    : s.screenshot}
                </span>{" "}
                · {selectedItem.app_name} · {formatRelativeTime(selectedItem.updated_at, lang)}
              </div>
              <button
                type="button"
                onClick={() => setSelectedItem(null)}
                className="w-6 h-6 rounded-full bg-white/10 hover:bg-white/20 text-white/75 hover:text-white flex items-center justify-center shrink-0"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>

            {/* Modal Body */}
            {selectedItem.kind === "clipboard_text" || selectedItem.kind === "fact" ? (
              <div className="flex-1 min-h-0 overflow-y-auto space-y-2 pr-0.5">
                <pre className="rounded-xl bg-black/60 border border-white/10 p-2.5 text-[11.5px] font-mono text-white/95 whitespace-pre-wrap break-words select-text leading-relaxed">
                  {selectedItem.content_text ?? ""}
                </pre>

                {selectedParsed && selectedParsed.items.length > 0 && (
                  <div className="space-y-1">
                    {selectedParsed.items.map((it, idx) => (
                      <div
                        key={idx}
                        className="rounded-lg bg-white/[0.05] border border-white/10 p-2 flex items-start justify-between gap-2"
                      >
                        <div className="min-w-0">
                          <div className="text-[9.5px] text-white/55">{it.label}</div>
                          <pre className="text-[10.5px] text-white/90 font-mono whitespace-pre-wrap break-all">
                            {it.value}
                          </pre>
                        </div>
                        <button
                          type="button"
                          onClick={() => {
                            void navigator.clipboard?.writeText(it.value).catch(() => {});
                          }}
                          className="p-1 text-white/55 hover:text-white shrink-0"
                        >
                          <Copy className="w-3 h-3" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <div className="flex-1 min-h-0 rounded-xl bg-black/80 border border-white/10 flex items-center justify-center overflow-auto p-1">
                {selectedItem.file_path && (
                  <MemoryImageView
                    path={selectedItem.file_path}
                    className="max-w-full max-h-full object-contain rounded"
                  />
                )}
              </div>
            )}

            {/* Modal Action Toolbar */}
            <div className="flex items-center justify-between gap-1.5 flex-wrap shrink-0 pt-1 border-t border-white/10">
              <button
                type="button"
                onClick={() => {
                  onAskAi(selectedItem);
                  setSelectedItem(null);
                }}
                className="px-2.5 py-1 rounded-lg bg-white/[0.14] hover:bg-white/[0.22] border border-white/15 text-[11px] font-medium text-white flex items-center gap-1.5 transition"
              >
                <BuddyMiniLogo />
                <span>{s.ask}</span>
              </button>

              <div className="flex items-center gap-1 flex-wrap">
                {selectedItem.kind !== "clipboard_text" &&
                  selectedItem.kind !== "fact" &&
                  selectedItem.file_path && (
                  <>
                    {enabledPlugins.offline_ocr !== false && (
                      <button
                        type="button"
                        onClick={() => handleOcr(selectedItem)}
                        className="px-2 py-1 rounded-lg bg-white/[0.07] hover:bg-white/15 text-[10.5px] text-white/85 flex items-center gap-1"
                      >
                        <ScanText className="w-3 h-3" />
                        <span>
                          {ocrDoneId === selectedItem.id ? "✓" : sp.ocrAction}
                        </span>
                      </button>
                    )}
                    {enabledPlugins.pin_to_screen !== false && (
                      <button
                        type="button"
                        onClick={() => {
                          void invoke("plugin_pin_image", {
                            source: selectedItem.file_path,
                          }).catch(() => {});
                        }}
                        className="px-2 py-1 rounded-lg bg-white/[0.07] hover:bg-white/15 text-[10.5px] text-white/85 flex items-center gap-1"
                      >
                        <Pin className="w-3 h-3" />
                        <span>{sp.pinAction}</span>
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() =>
                        void invoke("plugin_open_folder", { path: selectedItem.file_path })
                      }
                      title={s.openFolderTip}
                      className="p-1.5 rounded-lg bg-white/[0.07] hover:bg-white/15 text-white/75 hover:text-white"
                    >
                      <FolderOpen className="w-3.5 h-3.5" />
                    </button>
                  </>
                )}

                {selectedItem.kind === "clipboard_text" &&
                  selectedItem.content_text &&
                  enabledPlugins.clipboard_parser !== false && (
                    <button
                      type="button"
                      onClick={() => {
                        void invoke("plugin_clean_paste", {
                          text: selectedItem.content_text,
                        }).catch(() => {});
                      }}
                      className="px-2 py-1 rounded-lg bg-white/[0.07] hover:bg-white/15 text-[10.5px] text-white/85 flex items-center gap-1"
                    >
                      <Sparkles className="w-3 h-3" />
                      <span>{sp.cleanPaste}</span>
                    </button>
                  )}

                <button
                  type="button"
                  onClick={() => void handleCopy(selectedItem.id)}
                  className="px-2 py-1 rounded-lg bg-white/[0.07] hover:bg-white/15 text-[10.5px] text-white/85 flex items-center gap-1"
                >
                  {copiedId === selectedItem.id ? (
                    <Check className="w-3 h-3" />
                  ) : (
                    <Copy className="w-3 h-3" />
                  )}
                  <span>{copiedId === selectedItem.id ? "✓" : ""}</span>
                </button>
                <button
                  type="button"
                  onClick={() => void handleToggleStar(selectedItem.id)}
                  className="p-1.5 rounded-lg bg-white/[0.07] hover:bg-white/15"
                >
                  <Star
                    className={`w-3.5 h-3.5 ${
                      selectedItem.is_starred
                        ? "fill-amber-400 text-amber-400"
                        : "text-white/55"
                    }`}
                  />
                </button>
                <button
                  type="button"
                  onClick={() => void handleDelete(selectedItem.id)}
                  className="p-1.5 rounded-lg bg-white/[0.07] hover:bg-rose-500/25 text-white/55 hover:text-rose-300"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
