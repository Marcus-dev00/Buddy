import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, Pause, Play, RefreshCw, Square } from "lucide-react";
import { getStrings } from "../i18n";
import type {
  ModelProfile,
  ShareStatePayload,
  ShareTargetInfo,
  UiLang,
} from "../types";

export interface ScreenSharePanelProps {
  lang: UiLang;
  shareState: ShareStatePayload | null;
  modelProfiles: ModelProfile[];
  defaultWatchProfileId: string;
  onClose: () => void;
}

export function ScreenSharePanel({
  lang,
  shareState,
  modelProfiles,
  defaultWatchProfileId,
  onClose,
}: ScreenSharePanelProps) {
  const [shareTargets, setShareTargets] = useState<ShareTargetInfo[]>([]);
  const [selectedTargetId, setSelectedTargetId] = useState("auto_cursor");
  const [shareGoal, setShareGoal] = useState("");
  const [shareCadence, setShareCadence] = useState<"quiet" | "balanced" | "active">("balanced");
  const [shareShowBorder, setShareShowBorder] = useState(true);
  const [shareWatchId, setShareWatchId] = useState(defaultWatchProfileId || "");
  const [shareError, setShareError] = useState<string | null>(null);
  const [shareStarting, setShareStarting] = useState(false);
  const s = getStrings(lang).chat;

  const loadShareTargets = async () => {
    try {
      const list = await invoke<ShareTargetInfo[]>("list_share_targets");
      setShareTargets(list);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    void loadShareTargets();
  }, []);

  useEffect(() => {
    setShareWatchId((cur) => cur || defaultWatchProfileId || "");
  }, [defaultWatchProfileId]);

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
      onClose();
    } catch (err) {
      setShareError(String(err));
    } finally {
      setShareStarting(false);
    }
  };

  return (
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
                  onClose();
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
  );
}
