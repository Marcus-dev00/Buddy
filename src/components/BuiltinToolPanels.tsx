import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  Copy,
  Download,
  FolderOpen,
  Loader2,
  Pipette,
  Play,
  Square,
  Timer,
} from "lucide-react";
import { getStrings } from "../i18n";
import type {
  ClipboardParseResult,
  ColorPickedPayload,
  LockProcessInfo,
  OcrResult,
  PluginInfo,
  PluginToolOutput,
  TimerTickPayload,
  UiLang,
} from "../types";

function parseLines(raw: string): string[] {
  return raw
    .split(/\r?\n/)
    .map((x) => x.trim().replace(/^"(.*)"$/, "$1"))
    .filter(Boolean);
}

function ToolOutputBanner({
  output,
  openLabel,
}: {
  output: PluginToolOutput | null;
  openLabel: string;
}) {
  if (!output) return null;
  return (
    <div className="flex items-center justify-between text-[10px] text-emerald-300 bg-black/35 rounded-lg px-2 py-1">
      <span className="truncate">{output.summary}</span>
      {output.outputs[0] && (
        <button
          type="button"
          onClick={() => void invoke("plugin_open_folder", { path: output.outputs[0] })}
          className="shrink-0 ml-2 inline-flex items-center gap-1 text-sky-300 hover:underline"
        >
          <FolderOpen className="w-3 h-3" />
          <span>{openLabel}</span>
        </button>
      )}
    </div>
  );
}

export interface BuiltinToolPanelsProps {
  plugin: PluginInfo;
  pluginName: string;
  lang: UiLang;
  isRunning: boolean;
  lastPickedColor: ColorPickedPayload | null;
  timerState: TimerTickPayload | null;
  droppedPathsHint: string[];
  onCopyText: (text: string) => void;
  onError: (msg: string | null) => void;
}

export function BuiltinToolPanels({
  plugin: p,
  pluginName: name,
  lang,
  isRunning,
  lastPickedColor,
  timerState,
  droppedPathsHint,
  onCopyText,
  onError,
}: BuiltinToolPanelsProps) {
  const s = getStrings(lang).plugins;

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

  const [dlUrl, setDlUrl] = useState("");
  const [dlDir, setDlDir] = useState("");
  const [dlFmt, setDlFmt] = useState<"audio_best" | "mp4_lossless" | "audio_flac" | "audio_mp3">(
    "audio_best"
  );
  const [dlOut, setDlOut] = useState<PluginToolOutput | null>(null);

  const [copiedKey, setCopiedKey] = useState<string | null>(null);

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
    if (/\.(mp4|mov|mkv|webm|avi|m4v|flv|ts|wmv|gif|mp3|wav|flac|m4a)$/i.test(lower)) {
      setMediaPath(first);
    }
    if (/^https?:\/\//i.test(first.trim())) {
      setDlUrl(first.trim());
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

  const browseFiles = async (
    title: string,
    multiple: boolean,
    filterName: string,
    filterSpec: string
  ): Promise<string[]> => {
    try {
      return await invoke<string[]>("pick_files", {
        title,
        multiple,
        filterName,
        filterSpec,
        initialDir: null,
      });
    } catch (err) {
      onError(String(err));
      return [];
    }
  };

  const browseFolder = async (title: string, initialDir?: string): Promise<string | null> => {
    try {
      return await invoke<string | null>("pick_folder", {
        title,
        initialDir: initialDir?.trim() || null,
      });
    } catch (err) {
      onError(String(err));
      return null;
    }
  };

  if (p.id === "offline_ocr") {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <div className="flex gap-1.5">
          <input
            type="text"
            value={ocrPath}
            onChange={(e) => setOcrPath(e.target.value)}
            placeholder={s.ocrHint}
            className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
          />
          <button
            type="button"
            title={s.browseBtn}
            onClick={() => {
              void browseFiles(name, false, "Images", "*.png;*.jpg;*.jpeg;*.webp;*.bmp").then(
                (files) => {
                  if (files[0]) setOcrPath(files[0]);
                }
              );
            }}
            className="px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-white/80 transition"
          >
            <FolderOpen className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            disabled={ocrBusy || !ocrPath.trim()}
            onClick={async () => {
              setOcrBusy(true);
              onError(null);
              try {
                const res = await invoke<OcrResult>("plugin_offline_ocr", {
                  source: ocrPath.trim().replace(/^"(.*)"$/, "$1"),
                });
                setOcrRes(res);
              } catch (err) {
                onError(String(err));
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
    );
  }

  if (p.id === "pin_to_screen") {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] flex gap-1.5 items-center">
        <input
          type="text"
          value={pinPath}
          onChange={(e) => setPinPath(e.target.value)}
          placeholder="C:\...\image.png"
          className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
        />
        <button
          type="button"
          title={s.browseBtn}
          onClick={() => {
            void browseFiles(name, false, "Images", "*.png;*.jpg;*.jpeg;*.webp;*.bmp").then(
              (files) => {
                if (files[0]) setPinPath(files[0]);
              }
            );
          }}
          className="px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-white/80 transition"
        >
          <FolderOpen className="w-3.5 h-3.5" />
        </button>
        <button
          type="button"
          disabled={!pinPath.trim()}
          onClick={() => {
            onError(null);
            void invoke("plugin_pin_image", {
              source: pinPath.trim().replace(/^"(.*)"$/, "$1"),
            }).catch((e) => onError(String(e)));
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
    );
  }

  if (p.id === "clipboard_parser") {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <div className="flex gap-1.5">
          <button
            type="button"
            onClick={() => {
              void invoke<ClipboardParseResult>("plugin_parse_clipboard", { text: null })
                .then(setClipParsed)
                .catch((e) => onError(String(e)));
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
                    <div className="text-[9.5px] text-emerald-300 font-medium">{it.label}</div>
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
    );
  }

  if (p.id === "file_locksmith") {
    return (
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
              onError(null);
              void invoke<LockProcessInfo[]>("plugin_inspect_locks", {
                path: lockPath.trim().replace(/^"(.*)"$/, "$1"),
              })
                .then(setLockList)
                .catch((e) => onError(String(e)));
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
                        .catch((e) => onError(String(e)));
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
    );
  }

  if (p.id === "color_picker") {
    return (
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
            {[lastPickedColor.hex, lastPickedColor.rgb, lastPickedColor.hsl].map((val, i) => (
              <button
                key={i}
                type="button"
                onClick={() => markCopied(`clr_${i}`, val)}
                className="px-1.5 py-0.5 rounded bg-black/40 hover:bg-white/15 text-[10px] font-mono text-white/85"
              >
                {copiedKey === `clr_${i}` ? "✓" : val}
              </button>
            ))}
          </div>
        )}
      </div>
    );
  }

  if (p.id === "keep_awake_timer") {
    return (
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
              keepAwake ? "bg-orange-500 text-white" : "bg-white/10 text-white/75 hover:bg-white/20"
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
    );
  }

  if (p.id === "image_optimizer") {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <div className="flex gap-1.5 items-start">
          <textarea
            rows={2}
            value={imgPathsText}
            onChange={(e) => setImgPathsText(e.target.value)}
            placeholder={s.imgPathPlaceholder}
            className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 p-1.5 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none resize-none"
          />
          <button
            type="button"
            title={s.browseBtn}
            onClick={() => {
              void browseFiles(name, true, "Images", "*.png;*.jpg;*.jpeg;*.webp;*.bmp").then(
                (files) => {
                  if (files.length > 0) setImgPathsText(files.join("\n"));
                }
              );
            }}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/85 transition"
          >
            <FolderOpen className="w-3 h-3 text-sky-300" />
            <span>{s.browseBtn}</span>
          </button>
        </div>
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
              onError(null);
              void invoke<PluginToolOutput>("plugin_optimize_images", {
                paths: parseLines(imgPathsText),
                targetFormat: imgFmt,
                maxDim: Math.max(0, Number(imgMaxDim) || 0),
                quality: 84,
              })
                .then(setImgOut)
                .catch((e) => onError(String(e)));
            }}
            className="ml-auto px-2.5 py-1 rounded-lg bg-cyan-500/85 hover:bg-cyan-400 text-[10.5px] font-medium text-white"
          >
            {s.optimizeBtn}
          </button>
        </div>
        <ToolOutputBanner output={imgOut} openLabel={s.openFolder} />
      </div>
    );
  }

  if (p.id === "pdf_suite") {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <div className="flex gap-1.5 items-start">
          <textarea
            rows={2}
            value={pdfPathsText}
            onChange={(e) => setPdfPathsText(e.target.value)}
            placeholder={s.pdfPathPlaceholder}
            className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 p-1.5 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none resize-none"
          />
          <button
            type="button"
            title={s.browseBtn}
            onClick={() => {
              void browseFiles(
                name,
                true,
                "PDF, Docs & Images",
                "*.pdf;*.md;*.txt;*.docx;*.png;*.jpg;*.jpeg;*.webp"
              ).then((files) => {
                if (files.length > 0) setPdfPathsText(files.join("\n"));
              });
            }}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/85 transition"
          >
            <FolderOpen className="w-3 h-3 text-sky-300" />
            <span>{s.browseBtn}</span>
          </button>
        </div>
        <div className="flex items-center gap-1.5 flex-wrap">
          <button
            type="button"
            onClick={() => {
              onError(null);
              void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                action: "images_to_pdf",
                paths: parseLines(pdfPathsText),
                pageRange: null,
              })
                .then(setPdfOut)
                .catch((e) => onError(String(e)));
            }}
            className="px-2 py-0.5 rounded-lg bg-indigo-500/80 hover:bg-indigo-400 text-[10px] text-white"
          >
            {s.imagesToPdf}
          </button>
          <button
            type="button"
            onClick={() => {
              onError(null);
              void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                action: "doc_to_pdf",
                paths: parseLines(pdfPathsText),
                pageRange: null,
              })
                .then(setPdfOut)
                .catch((e) => onError(String(e)));
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
                  onError(null);
                  void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                    action: "merge",
                    paths: parseLines(pdfPathsText),
                    pageRange: null,
                  })
                    .then(setPdfOut)
                    .catch((e) => onError(String(e)));
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
                  onError(null);
                  void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                    action: "split",
                    paths: parseLines(pdfPathsText),
                    pageRange: pdfPages,
                  })
                    .then(setPdfOut)
                    .catch((e) => onError(String(e)));
                }}
                className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white"
              >
                {s.pdfSplit}
              </button>
              <button
                type="button"
                onClick={() => {
                  onError(null);
                  void invoke<PluginToolOutput>("plugin_run_pdf_tool", {
                    action: "compress",
                    paths: parseLines(pdfPathsText),
                    pageRange: null,
                  })
                    .then(setPdfOut)
                    .catch((e) => onError(String(e)));
                }}
                className="px-2 py-0.5 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white"
              >
                {s.pdfCompress}
              </button>
            </>
          )}
        </div>
        {!p.installed && (
          <div className="text-[10px] text-white/45 leading-snug">{s.pdfQpdfHint}</div>
        )}
        <ToolOutputBanner output={pdfOut} openLabel={s.openFolder} />
      </div>
    );
  }

  if (p.id === "media_gif" && p.installed) {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <div className="flex gap-1.5 items-center">
          <input
            type="text"
            value={mediaPath}
            onChange={(e) => setMediaPath(e.target.value)}
            placeholder={s.mediaPathPlaceholder}
            className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
          />
          <button
            type="button"
            title={s.browseBtn}
            onClick={() => {
              void browseFiles(
                name,
                false,
                "Video / Audio",
                "*.mp4;*.mkv;*.mov;*.webm;*.avi;*.mp3;*.wav;*.m4a"
              ).then((files) => {
                if (files[0]) setMediaPath(files[0]);
              });
            }}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/85 transition"
          >
            <FolderOpen className="w-3 h-3 text-sky-300" />
            <span>{s.browseBtn}</span>
          </button>
        </div>
        <div className="flex gap-1.5 flex-wrap items-center">
          {isRunning ? (
            <button
              type="button"
              onClick={() => void invoke("cancel_plugin_task")}
              className="px-2.5 py-1 rounded-lg bg-rose-500/75 hover:bg-rose-500 text-[10px] text-white flex items-center gap-1"
            >
              <Square className="w-2.5 h-2.5 fill-current" />
              <span>{s.cancelBtn}</span>
            </button>
          ) : (
            (
              [
                { act: "gif", label: s.toGif },
                { act: "compress", label: s.compressMp4 },
                { act: "audio", label: s.extractMp3 },
              ] as const
            ).map((m) => (
              <button
                key={m.act}
                type="button"
                disabled={!mediaPath.trim()}
                onClick={() => {
                  onError(null);
                  setMediaOut(null);
                  void invoke<PluginToolOutput>("plugin_run_media_tool", {
                    action: m.act,
                    inputPath: mediaPath.trim().replace(/^"(.*)"$/, "$1"),
                    maxWidth: 540,
                    fps: 12,
                  })
                    .then(setMediaOut)
                    .catch((e) => onError(String(e)));
                }}
                className="px-2.5 py-1 rounded-lg bg-violet-500/80 hover:bg-violet-400 disabled:opacity-40 text-[10px] text-white"
              >
                {m.label}
              </button>
            ))
          )}
        </div>
        <ToolOutputBanner output={mediaOut} openLabel={s.openFolder} />
      </div>
    );
  }

  if (p.id === "media_downloader" && p.installed) {
    return (
      <div className="pt-1.5 border-t border-white/[0.08] space-y-1.5">
        <input
          type="text"
          value={dlUrl}
          onChange={(e) => setDlUrl(e.target.value)}
          placeholder={s.dlUrlPlaceholder}
          className="w-full rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30 outline-none"
        />
        <div className="flex gap-1.5 items-center">
          <input
            type="text"
            value={dlDir}
            onChange={(e) => setDlDir(e.target.value)}
            placeholder={s.dlFolderPlaceholder}
            className="flex-1 min-w-0 rounded-lg bg-black/35 border border-white/10 px-2 py-1 text-[10px] font-mono text-white placeholder:text-white/30 outline-none"
          />
          <button
            type="button"
            title={s.browseBtn}
            onClick={() => {
              void browseFolder(name, dlDir).then((dir) => {
                if (dir) setDlDir(dir);
              });
            }}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-[10px] text-white/85 transition"
          >
            <FolderOpen className="w-3 h-3 text-sky-300" />
            <span>{s.browseBtn}</span>
          </button>
        </div>
        <div className="grid grid-cols-2 gap-1">
          {(
            [
              { id: "audio_best", label: s.dlFmtAudioBest },
              { id: "mp4_lossless", label: s.dlFmtMp4 },
              { id: "audio_flac", label: s.dlFmtFlac },
              { id: "audio_mp3", label: s.dlFmtMp3 },
            ] as const
          ).map((opt) => (
            <button
              key={opt.id}
              type="button"
              onClick={() => setDlFmt(opt.id)}
              className={`px-2 py-1 rounded-lg border text-[10px] font-medium transition ${
                dlFmt === opt.id
                  ? "bg-emerald-500/20 border-emerald-400/50 text-emerald-200"
                  : "bg-black/25 border-white/10 text-white/60 hover:text-white"
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
        <div className="text-[9.5px] text-white/45 leading-snug">{s.dlHint}</div>
        <div className="flex items-center gap-1.5">
          {isRunning ? (
            <button
              type="button"
              onClick={() => void invoke("cancel_plugin_task")}
              className="px-2.5 py-1 rounded-lg bg-rose-500/75 hover:bg-rose-500 text-[10px] text-white flex items-center gap-1"
            >
              <Square className="w-2.5 h-2.5 fill-current" />
              <span>{s.cancelBtn}</span>
            </button>
          ) : (
            <button
              type="button"
              disabled={!dlUrl.trim() || !p.enabled}
              onClick={() => {
                onError(null);
                setDlOut(null);
                void invoke<PluginToolOutput>("plugin_download_media", {
                  url: dlUrl.trim(),
                  format: dlFmt,
                  outputDir: dlDir.trim() ? dlDir.trim() : null,
                })
                  .then(setDlOut)
                  .catch((e) => onError(String(e)));
              }}
              className="px-2.5 py-1 rounded-lg bg-emerald-500/85 hover:bg-emerald-400 disabled:opacity-40 text-[10.5px] font-medium text-slate-950 flex items-center gap-1"
            >
              <Download className="w-3 h-3" />
              <span>{s.dlRun}</span>
            </button>
          )}
        </div>
        <ToolOutputBanner output={dlOut} openLabel={s.openFolder} />
      </div>
    );
  }

  return null;
}
