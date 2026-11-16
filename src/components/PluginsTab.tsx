import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Braces,
  Check,
  ChevronDown,
  ChevronRight,
  Coffee,
  Copy,
  Download,
  FileText,
  Film,
  FolderOpen,
  Globe,
  ImageDown,
  Loader2,
  Lock,
  Pin,
  Pipette,
  Play,
  ScanText,
  Search,
  Sparkles,
  Square,
  Timer,
  Trash2,
} from "lucide-react";
import { getStrings } from "../i18n";
import type {
  ClipboardParseResult,
  ColorPickedPayload,
  GithubSkillCandidate,
  LockProcessInfo,
  OcrResult,
  PluginInfo,
  PluginListResponse,
  PluginProgressPayload,
  PluginToolOutput,
  SkillDetail,
  SkillInfo,
  TimerTickPayload,
  UiLang,
} from "../types";

interface PluginsTabProps {
  lang: UiLang;
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

function PluginIcon({ id }: { id: string }) {
  const cls = "w-4 h-4 shrink-0";
  switch (id) {
    case "offline_ocr":
      return <ScanText className={`${cls} text-sky-400`} />;
    case "pin_to_screen":
      return <Pin className={`${cls} text-amber-400`} />;
    case "clipboard_parser":
      return <Braces className={`${cls} text-emerald-400`} />;
    case "file_locksmith":
      return <Lock className={`${cls} text-rose-400`} />;
    case "color_picker":
      return <Pipette className={`${cls} text-fuchsia-400`} />;
    case "keep_awake_timer":
      return <Coffee className={`${cls} text-orange-300`} />;
    case "image_optimizer":
      return <ImageDown className={`${cls} text-cyan-400`} />;
    case "pdf_suite":
      return <FileText className={`${cls} text-indigo-400`} />;
    case "media_gif":
      return <Film className={`${cls} text-violet-400`} />;
    default:
      return <FileText className={`${cls} text-white/70`} />;
  }
}

export default function PluginsTab({
  lang,
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
  const [openToolId, setOpenToolId] = useState<string | null>(
    focusedPluginId && focusedPluginId !== "__skills__" ? focusedPluginId : null
  );
  const [showMirror, setShowMirror] = useState(false);
  const [mirrorInput, setMirrorInput] = useState(pluginsData.mirror_prefix ?? "");

  // Skills state
  const [skillInput, setSkillInput] = useState("");
  const [skillBusy, setSkillBusy] = useState(false);
  const [ghResults, setGhResults] = useState<GithubSkillCandidate[]>([]);
  const [openSkillId, setOpenSkillId] = useState<string | null>(null);
  const [skillDetail, setSkillDetail] = useState<SkillDetail | null>(null);

  // Per-tool working state
  const [ocrPath, setOcrPath] = useState("");
  const [ocrRes, setOcrRes] = useState<OcrResult | null>(null);
  const [ocrBusy, setOcrBusy] = useState(false);

  const [pinPath, setPinPath] = useState("");
  const [clipParsed, setClipParsed] = useState<ClipboardParseResult | null>(null);
  const [lockPath, setLockPath] = useState("");
  const [lockList, setLockList] = useState<LockProcessInfo[] | null>(null);
  const [keepAwake, setKeepAwake] = useState(false);
  const [customMins, setCustomMins] = useState("25");

  const [imgPathsText, setImgPathsText] = useState("");
  const [imgFmt, setImgFmt] = useState<"jpg" | "png" | "webp">("webp");
  const [imgMaxDim, setImgMaxDim] = useState("1920");
  const [imgOut, setImgOut] = useState<PluginToolOutput | null>(null);

  const [pdfPathsText, setPdfPathsText] = useState("");
  const [pdfPages, setPdfPages] = useState("1");
  const [pdfOut, setPdfOut] = useState<PluginToolOutput | null>(null);

  const [mediaPath, setMediaPath] = useState("");
  const [mediaOut, setMediaOut] = useState<PluginToolOutput | null>(null);

  const [toolError, setToolError] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

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

  useEffect(() => {
    void invoke<boolean>("plugin_get_keep_awake").then(setKeepAwake).catch(() => {});
  }, []);

  useEffect(() => {
    if (droppedPathsHint.length === 0) return;
    const first = droppedPathsHint[0];
    const lower = first.toLowerCase();
    if (/\.(png|jpg|jpeg|webp|bmp)$/i.test(lower)) {
      setOcrPath(first);
      setPinPath(first);
      setImgPathsText(droppedPathsHint.join("\n"));
    }
    if (/\.(mp4|mov|mkv|webm|avi)$/i.test(lower)) {
      setMediaPath(first);
    }
    if (/\.(pdf|md|txt|doc|docx|ppt|pptx|xls|xlsx|png|jpg|jpeg)$/i.test(lower)) {
      setPdfPathsText(droppedPathsHint.join("\n"));
    }
    setLockPath(first);
  }, [droppedPathsHint]);

  const markCopied = (key: string, val: string) => {
    onCopyText(val);
    setCopiedKey(key);
    window.setTimeout(() => setCopiedKey(null), 1200);
  };

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

  const visiblePlugins = pluginsData.plugins.filter(
    (p) => filter === "all" || p.kind === filter
  );

  const parseLines = (raw: string) =>
    raw
      .split(/\r?\n/)
      .map((x) => x.trim().replace(/^"(.*)"$/, "$1"))
      .filter(Boolean);

  return (
    <div className="flex-1 flex flex-col min-h-0 space-y-2">
      {/* Top Filter & Mirror Row */}
      <div className="flex items-center justify-between gap-2 shrink-0">
        <div className="flex items-center gap-1 rounded-xl bg-white/[0.05] p-0.5 border border-white/10">
          {(
            [
              { id: "all", label: s.filterAll },
              { id: "native", label: s.filterNative },
              { id: "engine", label: s.filterEngine },
              { id: "skills", label: s.filterSkills },
            ] as const
          ).map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => setFilter(f.id)}
              className={`px-2 py-0.5 rounded-lg text-[10.5px] font-medium transition ${
                filter === f.id
                  ? "bg-white/15 text-white"
                  : "text-white/50 hover:text-white/85"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-2">
          {pluginsData.total_size_bytes > 0 && (
            <span className="text-[10px] font-mono text-white/45">
              {formatBytes(pluginsData.total_size_bytes)}
            </span>
          )}
          <button
            type="button"
            onClick={() => setShowMirror((v) => !v)}
            title={s.mirrorLabel}
            className={`p-1 rounded-lg border transition ${
              showMirror || pluginsData.mirror_prefix
                ? "bg-sky-500/20 border-sky-400/35 text-sky-300"
                : "bg-white/[0.05] border-white/10 text-white/55 hover:text-white"
            }`}
          >
            <Globe className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {showMirror && (
        <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2 space-y-1 shrink-0">
          <div className="text-[10px] text-white/55">{s.mirrorLabel}</div>
          <div className="flex gap-1.5">
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
              className="flex-1 rounded-lg bg-black/30 border border-white/10 focus:border-sky-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
            />
          </div>
        </div>
      )}

      {toolError && (
        <div className="rounded-xl bg-rose-500/15 border border-rose-400/30 px-2.5 py-1.5 text-[10.5px] text-rose-200 flex items-center justify-between gap-2 shrink-0">
          <span className="truncate">{toolError}</span>
          <button
            type="button"
            onClick={() => setToolError(null)}
            className="text-rose-200/70 hover:text-white text-[10px]"
          >
            ✕
          </button>
        </div>
      )}

      {/* Plugin Cards OR Skills Hub */}
      {filter === "skills" ? (
        <div className="flex-1 flex flex-col min-h-0 space-y-2">
          <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2 space-y-1.5 shrink-0">
            <div className="flex gap-1.5">
              <input
                type="text"
                value={skillInput}
                onChange={(e) => setSkillInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && skillInput.trim()) {
                    e.preventDefault();
                    const v = skillInput.trim();
                    const looksUrlOrRepo =
                      /^https?:\/\/github\.com\//i.test(v) || /^[a-z0-9_.-]+\/[a-z0-9_.-]+$/i.test(v);
                    const looksLocal = /^[a-z]:\\/i.test(v) || v.endsWith(".zip");
                    setToolError(null);
                    setSkillBusy(true);
                    const p = looksLocal
                      ? invoke<SkillInfo[]>("import_local_skill", { path: v })
                      : looksUrlOrRepo
                      ? invoke<SkillInfo[]>("install_github_skill_cmd", { spec: v })
                      : invoke<GithubSkillCandidate[]>("search_github_skills_cmd", { query: v }).then(
                          (cands) => {
                            setGhResults(cands);
                            return skills;
                          }
                        );
                    void p
                      .then((next) => {
                        if (looksLocal || looksUrlOrRepo) {
                          onRefreshSkills(next);
                          setSkillInput("");
                        }
                      })
                      .catch((err) => setToolError(String(err)))
                      .finally(() => setSkillBusy(false));
                  }
                }}
                placeholder={s.skillsHint}
                className="flex-1 rounded-lg bg-black/35 border border-white/10 focus:border-sky-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
              />
              <button
                type="button"
                disabled={skillBusy || !skillInput.trim()}
                onClick={() => {
                  setToolError(null);
                  setSkillBusy(true);
                  void invoke<GithubSkillCandidate[]>("search_github_skills_cmd", {
                    query: skillInput.trim(),
                  })
                    .then(setGhResults)
                    .catch((e) => setToolError(String(e)))
                    .finally(() => setSkillBusy(false));
                }}
                className="px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 disabled:opacity-40 text-[10px] text-white flex items-center gap-1 shrink-0"
              >
                <Search className="w-3 h-3" />
                <span>{s.skillsSearchBtn}</span>
              </button>
              <button
                type="button"
                disabled={skillBusy || !skillInput.trim()}
                onClick={() => {
                  const v = skillInput.trim();
                  const looksLocal = /^[a-z]:\\/i.test(v) || v.endsWith(".zip");
                  setToolError(null);
                  setSkillBusy(true);
                  const req = looksLocal
                    ? invoke<SkillInfo[]>("import_local_skill", { path: v })
                    : invoke<SkillInfo[]>("install_github_skill_cmd", { spec: v });
                  void req
                    .then((next) => {
                      onRefreshSkills(next);
                      setSkillInput("");
                    })
                    .catch((e) => setToolError(String(e)))
                    .finally(() => setSkillBusy(false));
                }}
                className="px-2 py-1 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-40 text-[10px] font-medium text-white flex items-center gap-1 shrink-0"
              >
                {skillBusy ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Download className="w-3 h-3" />
                )}
                <span>{s.skillsInstallBtn}</span>
              </button>
              <button
                type="button"
                onClick={() => void invoke("open_skills_folder", { id: null })}
                title={s.openFolder}
                className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/15 text-white/60 hover:text-white shrink-0"
              >
                <FolderOpen className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
            {ghResults.length > 0 && (
              <div className="space-y-1 pb-1 border-b border-white/10">
                {ghResults.map((cand, idx) => (
                  <div
                    key={idx}
                    className="rounded-xl bg-sky-500/[0.08] border border-sky-400/20 p-2 flex items-center justify-between gap-2"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5 text-[11px] font-medium text-sky-200 truncate">
                        <span>{cand.title}</span>
                        {cand.stars > 0 && (
                          <span className="text-[9.5px] font-mono text-amber-300/90">
                            ★{cand.stars}
                          </span>
                        )}
                      </div>
                      <div className="text-[10px] text-white/55 truncate">{cand.description}</div>
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      {cand.is_repo && (
                        <button
                          type="button"
                          disabled={skillBusy}
                          onClick={() => {
                            setToolError(null);
                            setSkillBusy(true);
                            void invoke<GithubSkillCandidate[]>("search_github_skills_cmd", {
                              query: cand.install_spec,
                            })
                              .then(setGhResults)
                              .catch((e) => setToolError(String(e)))
                              .finally(() => setSkillBusy(false));
                          }}
                          className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/85"
                        >
                          Browse
                        </button>
                      )}
                      <button
                        type="button"
                        disabled={skillBusy}
                        onClick={() => {
                          setToolError(null);
                          setSkillBusy(true);
                          void invoke<SkillInfo[]>("install_github_skill_cmd", {
                            spec: cand.install_spec,
                          })
                            .then((next) => {
                              onRefreshSkills(next);
                              setGhResults([]);
                            })
                            .catch((e) => setToolError(String(e)))
                            .finally(() => setSkillBusy(false));
                        }}
                        className="px-2 py-0.5 rounded-lg bg-sky-500 hover:bg-sky-400 text-[10px] font-medium text-white"
                      >
                        {s.skillsInstallBtn}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}

            {skills.length === 0 ? (
              <div className="rounded-xl bg-white/[0.03] border border-white/10 p-4 text-center text-[11px] text-white/45">
                {s.skillsEmpty}
              </div>
            ) : (
              skills.map((sk) => {
                const isOpen = openSkillId === sk.id;
                return (
                  <div
                    key={sk.id}
                    className="rounded-xl bg-white/[0.05] border border-white/10 p-2.5 space-y-1.5"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <button
                        type="button"
                        onClick={() => {
                          if (isOpen) {
                            setOpenSkillId(null);
                            setSkillDetail(null);
                          } else {
                            setOpenSkillId(sk.id);
                            void invoke<SkillDetail>("get_skill_detail", { id: sk.id })
                              .then(setSkillDetail)
                              .catch(() => {});
                          }
                        }}
                        className="flex items-start gap-2 min-w-0 text-left flex-1"
                      >
                        <Sparkles className="w-3.5 h-3.5 text-amber-300 mt-0.5 shrink-0" />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5">
                            <span className="text-[12px] font-medium text-white truncate">
                              {sk.name}
                            </span>
                            <span className="text-[9.5px] font-mono text-white/45">
                              {sk.file_count} files
                            </span>
                          </div>
                          <div className="text-[10.5px] text-white/50 line-clamp-2 mt-0.5">
                            {sk.description}
                          </div>
                        </div>
                      </button>

                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          onClick={() => void invoke("open_skills_folder", { id: sk.id })}
                          title={s.openFolder}
                          className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/15 text-white/50 hover:text-white"
                        >
                          <FolderOpen className="w-3 h-3" />
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            void invoke<SkillInfo[]>("uninstall_skill", { id: sk.id })
                              .then(onRefreshSkills)
                              .catch((e) => setToolError(String(e)));
                          }}
                          title={s.uninstallBtn}
                          className="p-1 rounded-lg bg-white/[0.06] hover:bg-rose-500/20 text-white/45 hover:text-rose-300"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                        <button
                          type="button"
                          role="switch"
                          aria-checked={sk.enabled}
                          onClick={() => {
                            void invoke<SkillInfo[]>("toggle_skill", {
                              id: sk.id,
                              enabled: !sk.enabled,
                            })
                              .then(onRefreshSkills)
                              .catch(() => {});
                          }}
                          className={`relative w-8 h-4 rounded-full transition-colors ${
                            sk.enabled ? "bg-sky-500" : "bg-white/20"
                          }`}
                        >
                          <span
                            className={`block w-3 h-3 rounded-full bg-white shadow transition-transform ${
                              sk.enabled ? "translate-x-4.5" : "translate-x-0.5"
                            }`}
                          />
                        </button>
                      </div>
                    </div>

                    {isOpen && skillDetail && skillDetail.info.id === sk.id && (
                      <div className="pt-1.5 border-t border-white/[0.08] space-y-1">
                        <div className="text-[9.5px] font-mono text-white/40 truncate">
                          {sk.source} · [{skillDetail.files.join(", ")}]
                        </div>
                        <pre className="rounded-lg bg-black/40 border border-white/10 p-2 text-[10px] font-mono text-white/85 whitespace-pre-wrap max-h-36 overflow-y-auto">
                          {skillDetail.skill_md}
                        </pre>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
        {visiblePlugins.map((p) => {
          const isOpen = openToolId === p.id;
          const prog = pluginProgress[p.id];
          const isDownloading = prog && (prog.stage === "downloading" || prog.stage === "extracting");
          const isRunning = prog && prog.stage === "running";
          const name = lang === "zh" ? p.name_zh : p.name_en;
          const desc = lang === "zh" ? p.desc_zh : p.desc_en;

          return (
            <div
              key={p.id}
              className="rounded-xl bg-white/[0.05] border border-white/10 p-2.5 space-y-2 transition"
            >
              <div className="flex items-start justify-between gap-2">
                <button
                  type="button"
                  onClick={() => setOpenToolId(isOpen ? null : p.id)}
                  className="flex items-start gap-2 min-w-0 text-left flex-1"
                >
                  <div className="mt-0.5">
                    <PluginIcon id={p.id} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 flex-wrap">
                      <span className="text-[12px] font-medium text-white truncate">{name}</span>
                      <span
                        className={`px-1.5 py-0.2 rounded text-[9.5px] font-medium ${
                          p.kind === "native"
                            ? "bg-emerald-500/15 text-emerald-300"
                            : p.installed
                            ? "bg-sky-500/20 text-sky-300"
                            : "bg-white/10 text-white/55"
                        }`}
                      >
                        {p.kind === "native"
                          ? s.nativeBadge
                          : p.installed
                          ? `${s.installedBadge} · ${formatBytes(p.size_bytes)}`
                          : p.engine_hint ?? s.notInstalledBadge}
                      </span>
                      {isOpen ? (
                        <ChevronDown className="w-3 h-3 text-white/40" />
                      ) : (
                        <ChevronRight className="w-3 h-3 text-white/40" />
                      )}
                    </div>
                    <div className="text-[10.5px] text-white/50 leading-snug mt-0.5">{desc}</div>
                  </div>
                </button>

                <div className="flex items-center gap-1.5 shrink-0 pt-0.5">
                  {p.kind === "engine" && !p.installed && !isDownloading && (
                    <button
                      type="button"
                      onClick={() => void handleInstall(p.id)}
                      className="inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-sky-500 hover:bg-sky-400 text-[10px] font-medium text-white transition"
                    >
                      <Download className="w-3 h-3" />
                      <span>{s.installBtn}</span>
                    </button>
                  )}
                  {p.kind === "engine" && isDownloading && (
                    <button
                      type="button"
                      onClick={() => void invoke("cancel_plugin_download")}
                      className="px-2 py-0.5 rounded-lg bg-rose-500/20 text-rose-200 text-[10px]"
                    >
                      {s.cancelBtn}
                    </button>
                  )}
                  {p.kind === "engine" && p.installed && (
                    <button
                      type="button"
                      onClick={() => void handleUninstall(p.id)}
                      title={s.uninstallBtn}
                      className="p-1 rounded-lg bg-white/[0.06] hover:bg-rose-500/20 text-white/45 hover:text-rose-300 transition"
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  )}
                  {(p.kind === "native" || p.installed) && (
                    <button
                      type="button"
                      role="switch"
                      aria-checked={p.enabled}
                      onClick={() => void handleToggle(p)}
                      className={`relative w-8 h-4 rounded-full transition-colors ${
                        p.enabled ? "bg-sky-500" : "bg-white/20"
                      }`}
                    >
                      <span
                        className={`block w-3 h-3 rounded-full bg-white shadow transition-transform ${
                          p.enabled ? "translate-x-4.5" : "translate-x-0.5"
                        }`}
                      />
                    </button>
                  )}
                </div>
              </div>

              {(isDownloading || isRunning) && (
                <div className="space-y-1">
                  <div className="flex justify-between text-[10px] text-sky-300 font-mono">
                    <span>{prog.detail}</span>
                    <span>{prog.percent}%</span>
                  </div>
                  <div className="h-1.5 rounded-full bg-black/40 overflow-hidden">
                    <div
                      className="h-full bg-sky-400 transition-all duration-150"
                      style={{ width: `${prog.percent}%` }}
                    />
                  </div>
                </div>
              )}

              {/* EXPANDED INLINE WORKBENCH PER PLUGIN */}
              {isOpen && p.id === "offline_ocr" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <div className="flex gap-1.5">
                    <input
                      type="text"
                      value={ocrPath}
                      onChange={(e) => setOcrPath(e.target.value)}
                      placeholder={s.ocrHint}
                      className="flex-1 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
                    />
                    <button
                      type="button"
                      disabled={ocrBusy || !ocrPath.trim()}
                      onClick={async () => {
                        setOcrBusy(true);
                        setToolError(null);
                        try {
                          const res = await invoke<OcrResult>("plugin_offline_ocr", {
                            source: ocrPath.trim().replace(/^"(.*)"$/, "$1"),
                          });
                          setOcrRes(res);
                        } catch (err) {
                          setToolError(String(err));
                        } finally {
                          setOcrBusy(false);
                        }
                      }}
                      className="px-2.5 py-1 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-40 text-[10.5px] font-medium text-white flex items-center gap-1"
                    >
                      {ocrBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Play className="w-3 h-3" />}
                      <span>{s.ocrAction}</span>
                    </button>
                  </div>
                  {ocrRes && (
                    <div className="rounded-lg bg-black/40 border border-white/10 p-2 space-y-1">
                      <div className="flex items-center justify-between text-[10px] text-white/45">
                        <span>
                          {ocrRes.language} · {ocrRes.line_count} lines
                        </span>
                        <button
                          type="button"
                          onClick={() => markCopied("ocr", ocrRes.text)}
                          className="text-sky-300 hover:text-sky-200 flex items-center gap-1"
                        >
                          {copiedKey === "ocr" ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                          <span>{s.ocrCopy}</span>
                        </button>
                      </div>
                      <pre className="text-[10.5px] text-white/90 whitespace-pre-wrap max-h-28 overflow-y-auto font-mono">
                        {ocrRes.text || "(no text detected)"}
                      </pre>
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "pin_to_screen" && (
                <div className="pt-1.5 border-t border-white/[0.08] flex gap-1.5 items-center">
                  <input
                    type="text"
                    value={pinPath}
                    onChange={(e) => setPinPath(e.target.value)}
                    placeholder="C:\...\image.png"
                    className="flex-1 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
                  />
                  <button
                    type="button"
                    disabled={!pinPath.trim()}
                    onClick={() => {
                      setToolError(null);
                      void invoke("plugin_pin_image", {
                        source: pinPath.trim().replace(/^"(.*)"$/, "$1"),
                      }).catch((e) => setToolError(String(e)));
                    }}
                    className="px-2.5 py-1 rounded-lg bg-amber-500/80 hover:bg-amber-400 disabled:opacity-40 text-[10.5px] font-medium text-white"
                  >
                    {s.pinAction}
                  </button>
                  <button
                    type="button"
                    onClick={() => void invoke("plugin_close_all_pins")}
                    className="px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/75"
                  >
                    {s.closeAllPins}
                  </button>
                </div>
              )}

              {isOpen && p.id === "clipboard_parser" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <div className="flex gap-1.5">
                    <button
                      type="button"
                      onClick={() => {
                        void invoke<ClipboardParseResult>("plugin_parse_clipboard", { text: null })
                          .then(setClipParsed)
                          .catch((e) => setToolError(String(e)));
                      }}
                      className="px-2.5 py-1 rounded-lg bg-emerald-500/80 hover:bg-emerald-400 text-[10.5px] font-medium text-white"
                    >
                      {s.parseAction}
                    </button>
                    {clipParsed && (
                      <button
                        type="button"
                        onClick={() => {
                          void invoke<string>("plugin_clean_paste", {
                            text: clipParsed.cleaned_text,
                          }).then((txt) => markCopied("clean", txt));
                        }}
                        className="px-2.5 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10.5px] text-white/85"
                      >
                        {copiedKey === "clean" ? "✓" : s.cleanPaste}
                      </button>
                    )}
                  </div>
                  {clipParsed && (
                    <div className="space-y-1 max-h-32 overflow-y-auto">
                      {clipParsed.items.length === 0 ? (
                        <div className="text-[10.5px] text-white/45">
                          {clipParsed.cleaned_text.slice(0, 120) || "Clipboard empty"}
                        </div>
                      ) : (
                        clipParsed.items.map((it, idx) => (
                          <div
                            key={idx}
                            className="rounded-lg bg-black/35 border border-white/10 p-1.5 flex items-start justify-between gap-2"
                          >
                            <div className="min-w-0">
                              <div className="text-[9.5px] text-emerald-300 font-medium">
                                {it.label}
                              </div>
                              <pre className="text-[10px] text-white/85 whitespace-pre-wrap break-all font-mono">
                                {it.value}
                              </pre>
                            </div>
                            <button
                              type="button"
                              onClick={() => markCopied(`clip_${idx}`, it.value)}
                              className="p-1 text-white/50 hover:text-white shrink-0"
                            >
                              {copiedKey === `clip_${idx}` ? (
                                <Check className="w-3 h-3 text-emerald-300" />
                              ) : (
                                <Copy className="w-3 h-3" />
                              )}
                            </button>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "file_locksmith" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <div className="flex gap-1.5">
                    <input
                      type="text"
                      value={lockPath}
                      onChange={(e) => setLockPath(e.target.value)}
                      placeholder={s.lockPathPlaceholder}
                      className="flex-1 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
                    />
                    <button
                      type="button"
                      disabled={!lockPath.trim()}
                      onClick={() => {
                        setToolError(null);
                        void invoke<LockProcessInfo[]>("plugin_inspect_locks", {
                          path: lockPath.trim().replace(/^"(.*)"$/, "$1"),
                        })
                          .then(setLockList)
                          .catch((e) => setToolError(String(e)));
                      }}
                      className="px-2.5 py-1 rounded-lg bg-rose-500/80 hover:bg-rose-400 disabled:opacity-40 text-[10.5px] font-medium text-white"
                    >
                      {s.checkLocks}
                    </button>
                  </div>
                  {lockList && (
                    <div className="space-y-1">
                      {lockList.length === 0 ? (
                        <div className="text-[10.5px] text-emerald-300/90">{s.noLocks}</div>
                      ) : (
                        lockList.map((proc) => (
                          <div
                            key={proc.pid}
                            className="flex items-center justify-between gap-2 rounded-lg bg-black/35 px-2 py-1 text-[10.5px]"
                          >
                            <div className="min-w-0 truncate text-white/85">
                              <span className="font-mono text-rose-300">PID {proc.pid}</span> ·{" "}
                              {proc.app_name}
                            </div>
                            <button
                              type="button"
                              onClick={() => {
                                void invoke("plugin_unlock_pid", { pid: proc.pid })
                                  .then(() =>
                                    invoke<LockProcessInfo[]>("plugin_inspect_locks", {
                                      path: lockPath.trim().replace(/^"(.*)"$/, "$1"),
                                    }).then(setLockList)
                                  )
                                  .catch((e) => setToolError(String(e)));
                              }}
                              className="px-2 py-0.5 rounded bg-rose-500/25 hover:bg-rose-500/45 text-rose-200 text-[10px]"
                            >
                              {s.unlockPid}
                            </button>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "color_picker" && (
                <div className="pt-1.5 border-t border-white/[0.08] flex items-center justify-between gap-2 flex-wrap">
                  <button
                    type="button"
                    onClick={() => void invoke("plugin_start_color_picker")}
                    className="px-2.5 py-1 rounded-lg bg-fuchsia-500/80 hover:bg-fuchsia-400 text-[10.5px] font-medium text-white flex items-center gap-1.5"
                  >
                    <Pipette className="w-3 h-3" />
                    <span>{s.pickColor}</span>
                  </button>
                  {lastPickedColor && (
                    <div className="flex items-center gap-1.5">
                      <span
                        className="w-4 h-4 rounded border border-white/25"
                        style={{ backgroundColor: lastPickedColor.hex }}
                      />
                      {[lastPickedColor.hex, lastPickedColor.rgb, lastPickedColor.hsl].map(
                        (val, i) => (
                          <button
                            key={i}
                            type="button"
                            onClick={() => markCopied(`clr_${i}`, val)}
                            className="px-1.5 py-0.5 rounded bg-black/40 hover:bg-white/15 text-[10px] font-mono text-white/85"
                          >
                            {copiedKey === `clr_${i}` ? "✓" : val}
                          </button>
                        )
                      )}
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "keep_awake_timer" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] text-white/85">
                      {keepAwake ? s.keepAwakeOn : s.keepAwake}
                    </span>
                    <button
                      type="button"
                      onClick={() => {
                        void invoke<boolean>("plugin_set_keep_awake", { enable: !keepAwake }).then(
                          setKeepAwake
                        );
                      }}
                      className={`px-2.5 py-0.5 rounded-lg text-[10.5px] font-medium transition ${
                        keepAwake
                          ? "bg-orange-500 text-white"
                          : "bg-white/10 text-white/75 hover:bg-white/20"
                      }`}
                    >
                      {keepAwake ? "ON" : "OFF"}
                    </button>
                  </div>
                  <div className="flex items-center gap-1.5 flex-wrap">
                    {[5, 15, 25].map((m) => (
                      <button
                        key={m}
                        type="button"
                        onClick={() =>
                          void invoke("plugin_start_timer", {
                            seconds: m * 60,
                            label: `${m}m`,
                          })
                        }
                        className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-sky-500/30 text-[10.5px] text-white/85 flex items-center gap-1"
                      >
                        <Timer className="w-3 h-3 text-sky-300" />
                        <span>{m}m</span>
                      </button>
                    ))}
                    <input
                      type="number"
                      min={1}
                      max={480}
                      value={customMins}
                      onChange={(e) => setCustomMins(e.target.value)}
                      className="w-12 rounded-lg bg-black/35 border border-white/10 px-1.5 py-0.5 text-[10.5px] font-mono text-white text-center outline-none"
                    />
                    <button
                      type="button"
                      onClick={() => {
                        const mins = Math.max(1, Number(customMins) || 1);
                        void invoke("plugin_start_timer", {
                          seconds: mins * 60,
                          label: `${mins}m`,
                        });
                      }}
                      className="px-2 py-0.5 rounded-lg bg-sky-500/80 hover:bg-sky-400 text-[10.5px] text-white"
                    >
                      {s.timerStart}
                    </button>
                    {timerState?.active && (
                      <button
                        type="button"
                        onClick={() => void invoke("plugin_cancel_timer")}
                        className="px-2 py-0.5 rounded-lg bg-rose-500/25 text-rose-200 text-[10px] flex items-center gap-1"
                      >
                        <Square className="w-2.5 h-2.5" />
                        <span>{s.timerStop}</span>
                      </button>
                    )}
                  </div>
                </div>
              )}

              {isOpen && p.id === "image_optimizer" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <textarea
                    rows={2}
                    value={imgPathsText}
                    onChange={(e) => setImgPathsText(e.target.value)}
                    placeholder={s.imgPathPlaceholder}
                    className="w-full rounded-lg bg-black/35 border border-white/10 p-1.5 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none resize-none"
                  />
                  <div className="flex items-center gap-1.5 flex-wrap">
                    {(["webp", "jpg", "png"] as const).map((fmt) => (
                      <button
                        key={fmt}
                        type="button"
                        onClick={() => setImgFmt(fmt)}
                        className={`px-2 py-0.5 rounded text-[10px] font-mono uppercase ${
                          imgFmt === fmt ? "bg-cyan-500 text-white" : "bg-white/10 text-white/65"
                        }`}
                      >
                        {fmt}
                      </button>
                    ))}
                    <input
                      type="number"
                      value={imgMaxDim}
                      onChange={(e) => setImgMaxDim(e.target.value)}
                      className="w-16 rounded-lg bg-black/35 border border-white/10 px-1.5 py-0.5 text-[10px] font-mono text-white text-center outline-none"
                      title="Max width/height px (0 = original)"
                    />
                    <button
                      type="button"
                      onClick={() => {
                        setToolError(null);
                        void invoke<PluginToolOutput>("plugin_optimize_images", {
                          paths: parseLines(imgPathsText),
                          targetFormat: imgFmt,
                          maxDim: Math.max(0, Number(imgMaxDim) || 0),
                          quality: 84,
                        })
                          .then(setImgOut)
                          .catch((e) => setToolError(String(e)));
                      }}
                      className="ml-auto px-2.5 py-1 rounded-lg bg-cyan-500/85 hover:bg-cyan-400 text-[10.5px] font-medium text-white"
                    >
                      {s.optimizeBtn}
                    </button>
                  </div>
                  {imgOut && (
                    <div className="flex items-center justify-between text-[10px] text-emerald-300 bg-black/35 rounded-lg px-2 py-1">
                      <span className="truncate">{imgOut.summary}</span>
                      {imgOut.outputs[0] && (
                        <button
                          type="button"
                          onClick={() =>
                            void invoke("plugin_open_folder", { path: imgOut.outputs[0] })
                          }
                          className="shrink-0 ml-2 inline-flex items-center gap-1 text-sky-300 hover:underline"
                        >
                          <FolderOpen className="w-3 h-3" />
                          <span>{s.openFolder}</span>
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "pdf_suite" && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <textarea
                    rows={2}
                    value={pdfPathsText}
                    onChange={(e) => setPdfPathsText(e.target.value)}
                    placeholder={s.pdfPathPlaceholder}
                    className="w-full rounded-lg bg-black/35 border border-white/10 p-1.5 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none resize-none"
                  />
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <button
                      type="button"
                      onClick={() => {
                        setToolError(null);
                        void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                          action: "images_to_pdf",
                          paths: parseLines(pdfPathsText),
                          pageRange: null,
                        })
                          .then(setPdfOut)
                          .catch((e) => setToolError(String(e)));
                      }}
                      className="px-2 py-0.5 rounded-lg bg-indigo-500/80 hover:bg-indigo-400 text-[10px] text-white"
                    >
                      {s.imagesToPdf}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setToolError(null);
                        void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                          action: "doc_to_pdf",
                          paths: parseLines(pdfPathsText),
                          pageRange: null,
                        })
                          .then(setPdfOut)
                          .catch((e) => setToolError(String(e)));
                      }}
                      className="px-2 py-0.5 rounded-lg bg-indigo-500/80 hover:bg-indigo-400 text-[10px] text-white"
                    >
                      {s.docToPdf}
                    </button>
                    {p.installed && (
                      <>
                        <button
                          type="button"
                          onClick={() => {
                            setToolError(null);
                            void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                              action: "merge",
                              paths: parseLines(pdfPathsText),
                              pageRange: null,
                            })
                              .then(setPdfOut)
                              .catch((e) => setToolError(String(e)));
                          }}
                          className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white"
                        >
                          {s.pdfMerge}
                        </button>
                        <input
                          type="text"
                          value={pdfPages}
                          onChange={(e) => setPdfPages(e.target.value)}
                          placeholder="1-3"
                          className="w-12 rounded bg-black/35 border border-white/10 px-1 py-0.5 text-[10px] font-mono text-white text-center"
                        />
                        <button
                          type="button"
                          onClick={() => {
                            setToolError(null);
                            void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                              action: "split",
                              paths: parseLines(pdfPathsText),
                              pageRange: pdfPages,
                            })
                              .then(setPdfOut)
                              .catch((e) => setToolError(String(e)));
                          }}
                          className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white"
                        >
                          {s.pdfSplit}
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            setToolError(null);
                            void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                              action: "compress",
                              paths: parseLines(pdfPathsText),
                              pageRange: null,
                            })
                              .then(setPdfOut)
                              .catch((e) => setToolError(String(e)));
                          }}
                          className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white"
                        >
                          {s.pdfCompress}
                        </button>
                      </>
                    )}
                  </div>
                  {pdfOut && (
                    <div className="flex items-center justify-between text-[10px] text-emerald-300 bg-black/35 rounded-lg px-2 py-1">
                      <span className="truncate">{pdfOut.summary}</span>
                      {pdfOut.outputs[0] && (
                        <button
                          type="button"
                          onClick={() =>
                            void invoke("plugin_open_folder", { path: pdfOut.outputs[0] })
                          }
                          className="shrink-0 ml-2 inline-flex items-center gap-1 text-sky-300 hover:underline"
                        >
                          <FolderOpen className="w-3 h-3" />
                          <span>{s.openFolder}</span>
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}

              {isOpen && p.id === "media_gif" && p.installed && (
                <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
                  <input
                    type="text"
                    value={mediaPath}
                    onChange={(e) => setMediaPath(e.target.value)}
                    placeholder={s.mediaPathPlaceholder}
                    className="w-full rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
                  />
                  <div className="flex gap-1.5 flex-wrap">
                    {(
                      [
                        { act: "gif", label: s.toGif },
                        { act: "compress", label: s.compressMp4 },
                        { act: "audio", label: s.extractMp3 },
                      ] as const
                    ).map((m) => (
                      <button
                        key={m.act}
                        type="button"
                        disabled={!mediaPath.trim() || Boolean(isRunning)}
                        onClick={() => {
                          setToolError(null);
                          void invoke<PluginToolOutput>("plugin_run_media_tool", {
                            action: m.act,
                            inputPath: mediaPath.trim().replace(/^"(.*)"$/, "$1"),
                            maxWidth: 540,
                            fps: 12,
                          })
                            .then(setMediaOut)
                            .catch((e) => setToolError(String(e)));
                        }}
                        className="px-2.5 py-1 rounded-lg bg-violet-500/80 hover:bg-violet-400 disabled:opacity-40 text-[10px] text-white"
                      >
                        {m.label}
                      </button>
                    ))}
                  </div>
                  {mediaOut && (
                    <div className="flex items-center justify-between text-[10px] text-emerald-300 bg-black/35 rounded-lg px-2 py-1">
                      <span className="truncate">{mediaOut.summary}</span>
                      {mediaOut.outputs[0] && (
                        <button
                          type="button"
                          onClick={() =>
                            void invoke("plugin_open_folder", { path: mediaOut.outputs[0] })
                          }
                          className="shrink-0 ml-2 inline-flex items-center gap-1 text-sky-300 hover:underline"
                        >
                          <FolderOpen className="w-3 h-3" />
                          <span>{s.openFolder}</span>
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        </div>
      )}
    </div>
  );
}
