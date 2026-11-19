import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Braces,
  ChevronDown,
  Coffee,
  Download,
  FileText,
  Film,
  Globe,
  HardDrive,
  ImageDown,
  Lock,
  Music,
  Pin,
  Pipette,
  ScanText,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { getStrings } from "../i18n";
import { BuiltinToolPanels } from "./BuiltinToolPanels";
import { SkillsSection } from "./SkillsSection";
import type {
  ColorPickedPayload,
  PluginInfo,
  PluginListResponse,
  PluginProgressPayload,
  SkillInfo,
  TimerTickPayload,
  UiLang,
} from "../types";

interface PluginsTabProps {
  lang: UiLang;
  isWindowMode?: boolean;
  pluginsData: PluginListResponse;
  pluginProgress: Record<string, PluginProgressPayload>;
  lastPickedColor: ColorPickedPayload | null;
  timerState: TimerTickPayload | null;
  droppedPathsHint: string[];
  focusedPluginId?: string | null;
  skills: SkillInfo[];
  onRefreshPlugins: (next: PluginListResponse) => void;
  onRefreshSkills: (next: SkillInfo[]) => void;
  onCopyText: (text: string) => void;
}

type PluginFilter = "all" | "native" | "engine" | "skills";

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb < 0.1) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${mb.toFixed(1)} MB`;
}

function getPluginTheme(id: string): { icon: ReactNode; medallion: string } {
  const cls = "w-4 h-4 shrink-0";
  switch (id) {
    case "offline_ocr":
      return {
        icon: <ScanText className={`${cls} text-sky-300`} />,
        medallion: "bg-sky-500/14 border-sky-400/25",
      };
    case "pin_to_screen":
      return {
        icon: <Pin className={`${cls} text-amber-300`} />,
        medallion: "bg-amber-500/14 border-amber-400/25",
      };
    case "clipboard_parser":
      return {
        icon: <Braces className={`${cls} text-emerald-300`} />,
        medallion: "bg-emerald-500/14 border-emerald-400/25",
      };
    case "file_locksmith":
      return {
        icon: <Lock className={`${cls} text-rose-300`} />,
        medallion: "bg-rose-500/14 border-rose-400/25",
      };
    case "color_picker":
      return {
        icon: <Pipette className={`${cls} text-fuchsia-300`} />,
        medallion: "bg-fuchsia-500/14 border-fuchsia-400/25",
      };
    case "keep_awake_timer":
      return {
        icon: <Coffee className={`${cls} text-orange-300`} />,
        medallion: "bg-orange-500/14 border-orange-400/25",
      };
    case "image_optimizer":
      return {
        icon: <ImageDown className={`${cls} text-cyan-300`} />,
        medallion: "bg-cyan-500/14 border-cyan-400/25",
      };
    case "pdf_suite":
      return {
        icon: <FileText className={`${cls} text-indigo-300`} />,
        medallion: "bg-indigo-500/14 border-indigo-400/25",
      };
    case "media_gif":
      return {
        icon: <Film className={`${cls} text-violet-300`} />,
        medallion: "bg-violet-500/14 border-violet-400/25",
      };
    case "media_downloader":
      return {
        icon: <Music className={`${cls} text-emerald-300`} />,
        medallion: "bg-emerald-500/14 border-emerald-400/25",
      };
    default:
      return {
        icon: <FileText className={`${cls} text-white/75`} />,
        medallion: "bg-white/[0.08] border-white/15",
      };
  }
}

export default function PluginsTab({
  lang,
  isWindowMode = false,
  pluginsData,
  pluginProgress,
  lastPickedColor,
  timerState,
  droppedPathsHint,
  focusedPluginId,
  skills,
  onRefreshPlugins,
  onRefreshSkills,
  onCopyText,
}: PluginsTabProps) {
  const s = getStrings(lang).plugins;
  const [filter, setFilter] = useState<PluginFilter>(
    focusedPluginId === "__skills__" ? "skills" : "all"
  );
  const [searchQuery, setSearchQuery] = useState("");
  const [openToolId, setOpenToolId] = useState<string | null>(
    focusedPluginId && focusedPluginId !== "__skills__" ? focusedPluginId : null
  );
  const [showMirror, setShowMirror] = useState(false);
  const [mirrorInput, setMirrorInput] = useState(pluginsData.mirror_prefix ?? "");
  const [toolError, setToolError] = useState<string | null>(null);

  useEffect(() => {
    setMirrorInput(pluginsData.mirror_prefix ?? "");
  }, [pluginsData.mirror_prefix]);

  useEffect(() => {
    if (focusedPluginId === "__skills__") {
      setFilter("skills");
    } else if (focusedPluginId) {
      setOpenToolId(focusedPluginId);
      if (filter === "skills") setFilter("all");
    }
  }, [focusedPluginId]);

  const handleToggle = async (p: PluginInfo) => {
    const next = await invoke<PluginListResponse>("toggle_plugin", {
      id: p.id,
      enabled: !p.enabled,
    });
    onRefreshPlugins(next);
  };

  const handleInstall = async (id: string) => {
    setToolError(null);
    try {
      const next = await invoke<PluginListResponse>("install_plugin_engine", { id });
      onRefreshPlugins(next);
    } catch (err) {
      setToolError(String(err));
    }
  };

  const handleUninstall = async (id: string) => {
    setToolError(null);
    try {
      const next = await invoke<PluginListResponse>("uninstall_plugin_engine", { id });
      onRefreshPlugins(next);
    } catch (err) {
      setToolError(String(err));
    }
  };

  const nativeCount = pluginsData.plugins.filter((p) => p.kind === "native").length;
  const engineCount = pluginsData.plugins.filter((p) => p.kind === "engine").length;

  const q = searchQuery.trim().toLowerCase();
  const matchesQuery = (p: PluginInfo) => {
    if (!q) return true;
    return (
      p.name_zh.toLowerCase().includes(q) ||
      p.name_en.toLowerCase().includes(q) ||
      p.desc_zh.toLowerCase().includes(q) ||
      p.desc_en.toLowerCase().includes(q) ||
      p.id.toLowerCase().includes(q)
    );
  };

  const visibleNative = pluginsData.plugins.filter(
    (p) => p.kind === "native" && (filter === "all" || filter === "native") && matchesQuery(p)
  );
  const visibleEngines = pluginsData.plugins.filter(
    (p) => p.kind === "engine" && (filter === "all" || filter === "engine") && matchesQuery(p)
  );
  const hasAnyPlugin = visibleNative.length > 0 || visibleEngines.length > 0;

  const renderPluginCard = (p: PluginInfo) => {
    const isOpen = openToolId === p.id;
    const prog = pluginProgress[p.id];
    const isDownloading = prog && (prog.stage === "downloading" || prog.stage === "extracting");
    const isRunning = prog && prog.stage === "running";
    const name = lang === "zh" ? p.name_zh : p.name_en;
    const desc = lang === "zh" ? p.desc_zh : p.desc_en;
    const theme = getPluginTheme(p.id);
    const isDimmed = (p.kind === "native" || p.installed) && !p.enabled && !isOpen;

    return (
      <div
        key={p.id}
        className={`rounded-2xl border transition-all duration-200 ${
          isOpen
            ? "bg-gradient-to-b from-white/[0.08] to-white/[0.04] border-sky-400/30 shadow-[0_8px_24px_rgba(0,0,0,0.28)]"
            : "bg-white/[0.035] hover:bg-white/[0.06] border-white/[0.09] hover:border-white/[0.15]"
        } ${isDimmed ? "opacity-60 hover:opacity-90" : ""}`}
      >
        <div
          onClick={() => setOpenToolId(isOpen ? null : p.id)}
          className="px-3 py-2.5 flex items-center gap-3 cursor-pointer select-none"
        >
          <div
            className={`w-9 h-9 rounded-xl border flex items-center justify-center shrink-0 transition-transform duration-200 ${theme.medallion} ${
              isOpen ? "scale-[1.03]" : ""
            }`}
          >
            {theme.icon}
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-1.5 min-w-0">
              <span className="text-[12.5px] font-semibold text-white truncate">{name}</span>
              {p.kind === "engine" && p.installed && (
                <span className="px-1.5 py-[1px] rounded-md bg-sky-500/15 border border-sky-400/20 text-[9px] font-mono text-sky-300 shrink-0">
                  {formatBytes(p.size_bytes)}
                </span>
              )}
              {p.kind === "engine" && !p.installed && (
                <span className="px-1.5 py-[1px] rounded-md bg-white/[0.06] border border-white/10 text-[9px] text-white/55 shrink-0">
                  {p.id === "pdf_suite" ? s.pdfBuiltInBadge : p.engine_hint ?? s.notInstalledBadge}
                </span>
              )}
            </div>
            <p className="text-[10.5px] text-white/50 leading-snug mt-0.5 line-clamp-1">
              {desc}
            </p>
          </div>

          <div
            className="flex items-center gap-1.5 shrink-0"
            onClick={(e) => e.stopPropagation()}
          >
            {p.kind === "engine" && !p.installed && !isDownloading && (
              <button
                type="button"
                onClick={() => void handleInstall(p.id)}
                className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full bg-sky-500 hover:bg-sky-400 text-[10px] font-semibold text-slate-950 shadow-sm transition"
              >
                <Download className="w-3 h-3" />
                <span>{s.installBtn}</span>
              </button>
            )}

            {p.kind === "engine" && isDownloading && (
              <button
                type="button"
                onClick={() => void invoke("cancel_plugin_download")}
                className="px-2.5 py-1 rounded-full bg-rose-500/20 hover:bg-rose-500/35 border border-rose-400/30 text-rose-200 text-[10px] transition"
              >
                {s.cancelBtn}
              </button>
            )}

            {p.kind === "engine" && p.installed && (
              <button
                type="button"
                onClick={() => void handleUninstall(p.id)}
                title={s.uninstallBtn}
                className="p-1.5 rounded-lg text-white/35 hover:text-rose-300 hover:bg-rose-500/15 transition"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            )}

            {(p.kind === "native" || p.installed) && (
              <button
                type="button"
                role="switch"
                aria-checked={p.enabled}
                onClick={() => void handleToggle(p)}
                className={`relative w-8 h-[18px] rounded-full transition-colors ${
                  p.enabled ? "bg-sky-500 shadow-[0_0_10px_rgba(14,165,233,0.35)]" : "bg-white/20"
                }`}
              >
                <span
                  className={`block w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
                    p.enabled ? "translate-x-[16px]" : "translate-x-[2px]"
                  }`}
                />
              </button>
            )}

            <button
              type="button"
              onClick={() => setOpenToolId(isOpen ? null : p.id)}
              aria-label="Toggle workbench"
              className="p-1 rounded-lg text-white/40 hover:text-white/80 transition"
            >
              <ChevronDown
                className={`w-3.5 h-3.5 transition-transform duration-200 ${
                  isOpen ? "rotate-180 text-sky-300" : ""
                }`}
              />
            </button>
          </div>
        </div>

        {(isDownloading || isRunning) && (
          <div className="px-3 pb-2.5 space-y-1">
            <div className="flex justify-between text-[10px] text-sky-300 font-mono">
              <span className="truncate pr-2">{prog.detail}</span>
              <span className="shrink-0">{prog.percent}%</span>
            </div>
            <div className="h-1.5 rounded-full bg-black/45 overflow-hidden">
              <div
                className="h-full bg-gradient-to-r from-sky-500 to-cyan-300 transition-all duration-150"
                style={{ width: `${prog.percent}%` }}
              />
            </div>
          </div>
        )}

        {isOpen && (
          <div className="px-2.5 pb-2.5">
            <div className="rounded-xl bg-black/25 border border-white/[0.07] p-2.5">
              <BuiltinToolPanels
                plugin={p}
                pluginName={name}
                lang={lang}
                isRunning={Boolean(isRunning)}
                lastPickedColor={lastPickedColor}
                timerState={timerState}
                droppedPathsHint={droppedPathsHint}
                onCopyText={onCopyText}
                onError={setToolError}
              />
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <div
      className={`flex-1 flex flex-col min-h-0 ${
        isWindowMode ? "max-w-5xl w-full mx-auto space-y-4" : "space-y-2"
      }`}
    >
      {/* Top Segmented Tabs + Search (inline in Window Mode) + Storage & Mirror */}
      <div className="flex items-center justify-between gap-2 shrink-0">
        <div className="flex items-center gap-0.5 rounded-full bg-white/[0.045] p-0.5 border border-white/10 min-w-0 overflow-x-auto [&::-webkit-scrollbar]:hidden">
          {(
            [
              { id: "all", label: s.filterAll, count: pluginsData.plugins.length },
              { id: "native", label: s.filterNative, count: nativeCount },
              { id: "engine", label: s.filterEngine, count: engineCount },
              { id: "skills", label: s.filterSkills, count: skills.length },
            ] as const
          ).map((f) => {
            const active = filter === f.id;
            return (
              <button
                key={f.id}
                type="button"
                onClick={() => setFilter(f.id)}
                className={`shrink-0 whitespace-nowrap rounded-full border font-medium flex items-center gap-1 transition-all duration-200 ${
                  isWindowMode ? "px-3.5 py-1 text-[12.5px]" : "px-2 py-1 text-[10.5px]"
                } ${
                  active
                    ? "bg-white text-black border-white"
                    : "border-transparent text-white/55 hover:text-white/90 hover:bg-white/[0.05]"
                }`}
              >
                <span>{f.label}</span>
                <span
                  className={`text-[9.5px] font-mono ${
                    active ? "text-black/65" : "text-white/35"
                  }`}
                >
                  {f.count}
                </span>
              </button>
            );
          })}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {isWindowMode && filter !== "skills" && (
            <div className="relative w-60">
              <Search className="w-3.5 h-3.5 text-white/35 absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={s.searchPluginsPlaceholder}
                className="w-full rounded-full bg-white/[0.05] border border-white/10 focus:border-white/25 outline-none pl-8 pr-8 py-1.5 text-[12.5px] text-white placeholder:text-white/35 transition"
              />
              {searchQuery && (
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-white/40 hover:text-white"
                >
                  <X className="w-3 h-3" />
                </button>
              )}
            </div>
          )}
          {pluginsData.total_size_bytes > 0 && (
            <span
              title={s.diskUsed}
              className="px-2 py-0.5 rounded-full bg-white/[0.04] border border-white/10 text-[10px] font-mono text-white/50 inline-flex items-center gap-1 whitespace-nowrap"
            >
              <HardDrive className="w-3 h-3 text-white/40" />
              {formatBytes(pluginsData.total_size_bytes)}
            </span>
          )}
          <button
            type="button"
            onClick={() => setShowMirror((v) => !v)}
            title={s.mirrorLabel}
            className={`w-7 h-7 rounded-full border flex items-center justify-center transition ${
              showMirror || pluginsData.mirror_prefix
                ? "bg-white/15 border-white/25 text-white"
                : "bg-white/[0.05] border-white/10 text-white/55 hover:text-white"
            }`}
          >
            <Globe className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {showMirror && (
        <div className="rounded-2xl bg-white/[0.045] border border-white/10 p-2.5 space-y-1.5 shrink-0">
          <div className="text-[10px] text-white/60">{s.mirrorLabel}</div>
          <input
            type="text"
            value={mirrorInput}
            onChange={(e) => setMirrorInput(e.target.value)}
            onBlur={() => {
              void invoke<PluginListResponse>("set_plugin_mirror", { prefix: mirrorInput })
                .then(onRefreshPlugins)
                .catch(() => {});
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
            placeholder={s.mirrorPlaceholder}
            className="w-full rounded-xl bg-black/35 border border-white/10 focus:border-sky-400/50 outline-none px-2.5 py-1.5 text-[10.5px] font-mono text-white placeholder:text-white/30"
          />
        </div>
      )}

      {toolError && (
        <div className="rounded-2xl bg-rose-500/15 border border-rose-400/30 px-3 py-2 text-[10.5px] text-rose-200 flex items-start justify-between gap-2 shrink-0">
          <span className="break-words whitespace-pre-wrap max-h-20 overflow-y-auto flex-1">
            {toolError}
          </span>
          <button
            type="button"
            onClick={() => setToolError(null)}
            className="text-rose-200/70 hover:text-white text-[10px] shrink-0"
          >
            ✕
          </button>
        </div>
      )}

      {/* Search bar in compact Island mode */}
      {!isWindowMode && filter !== "skills" && (
        <div className="relative shrink-0">
          <Search className="w-3.5 h-3.5 text-white/35 absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={s.searchPluginsPlaceholder}
            className="w-full rounded-full bg-white/[0.05] border border-white/10 focus:border-white/25 outline-none pl-8 pr-8 py-1.5 text-[11.5px] text-white placeholder:text-white/35 transition"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-white/40 hover:text-white"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      )}

      {/* Plugin Cards OR Skills Hub */}
      {filter === "skills" ? (
        <SkillsSection
          lang={lang}
          skills={skills}
          onRefreshSkills={onRefreshSkills}
          onError={setToolError}
        />
      ) : (
        <div className="flex-1 overflow-y-auto space-y-3 pr-1">
          {!hasAnyPlugin ? (
            <div className="h-full flex items-center justify-center text-[11.5px] text-white/40 py-8">
              {s.noPluginMatches}
            </div>
          ) : (
            <>
              {visibleNative.length > 0 && (
                <div className="space-y-1.5">
                  {filter === "all" && (
                    <div className="flex items-center gap-1.5 px-1 pt-0.5 text-[10px] font-medium uppercase tracking-wider text-white/45">
                      <span className="w-1.5 h-1.5 rounded-full bg-emerald-400/80" />
                      <span>
                        {s.sectionNative} · {visibleNative.length}
                      </span>
                    </div>
                  )}
                  <div
                    className={
                      isWindowMode
                        ? "grid grid-cols-1 md:grid-cols-2 gap-3 items-start"
                        : "space-y-1.5"
                    }
                  >
                    {visibleNative.map(renderPluginCard)}
                  </div>
                </div>
              )}

              {visibleEngines.length > 0 && (
                <div className="space-y-1.5">
                  {filter === "all" && (
                    <div className="flex items-center gap-1.5 px-1 pt-1 text-[10px] font-medium uppercase tracking-wider text-white/45">
                      <span className="w-1.5 h-1.5 rounded-full bg-sky-400/80" />
                      <span>
                        {s.sectionEngine} · {visibleEngines.length}
                      </span>
                    </div>
                  )}
                  <div
                    className={
                      isWindowMode
                        ? "grid grid-cols-1 md:grid-cols-2 gap-3 items-start"
                        : "space-y-1.5"
                    }
                  >
                    {visibleEngines.map(renderPluginCard)}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
