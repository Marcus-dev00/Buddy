import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Camera,
  Cpu,
  Download,
  FolderOpen,
  Loader2,
  Plug,
  RotateCcw,
  Sliders,
  Smartphone,
  Sparkles,
  Zap,
} from "lucide-react";
import { getStrings } from "../i18n";
import {
  clampThinkingLevel,
  formatThinkingBadge,
  getModelThinkingCapability,
  normalizeThinkingLevel,
  resolveActiveChatModel,
} from "../thinking";
import { getStepDisplay, ThinkingSlider } from "./ChatTab";
import { McpSection } from "./McpSection";
import { MobileLinkCard } from "./MobileLinkCard";
import { ModelProfilesSection } from "./ModelProfilesSection";
import { DarkSelect, LANGUAGE_OPTIONS, ToggleSwitch } from "./SettingsControls";
import type { AppSettings, McpServerStatus, UiLang, UpdateInfo } from "../types";

interface SettingsTabProps {
  isWindowMode?: boolean;
  settings: AppSettings;
  refreshHz: number;
  expandedDims: [number, number];
  appVersion: string;
  availableUpdate: UpdateInfo | null;
  updateProgress: number | null;
  onSaved: (updated: AppSettings) => void;
  onTriggerSnip: () => void;
  onTestInhale: () => void;
  onCheckUpdate: () => Promise<{ info: UpdateInfo | null; error?: string }>;
  onInstallUpdate: () => void;
  onResetDock: () => void;
}

type SettingsSection = "model" | "general" | "mcp" | "mobile" | "actions";

export default function SettingsTab({
  isWindowMode = false,
  settings,
  refreshHz,
  expandedDims,
  appVersion,
  availableUpdate,
  updateProgress,
  onSaved,
  onTriggerSnip,
  onTestInhale,
  onCheckUpdate,
  onInstallUpdate,
  onResetDock,
}: SettingsTabProps) {
  const [section, setSection] = useState<SettingsSection>("model");
  const [draft, setDraft] = useState<AppSettings>(settings);
  const [statusMsg, setStatusMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [updateMsg, setUpdateMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [browserCleared, setBrowserCleared] = useState(false);
  const [mcpServers, setMcpServers] = useState<McpServerStatus[]>([]);
  const [mcpError, setMcpError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<McpServerStatus[]>("list_mcp_servers")
      .then(setMcpServers)
      .catch(() => {});
  }, []);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  const lang: UiLang = draft.ui_language === "zh" ? "zh" : "en";
  const s = getStrings(lang).settings;

  const handleSaveDirect = async (nextDraft = draft, showBanner = true) => {
    if (showBanner) setStatusMsg(null);
    try {
      const updated = await invoke<AppSettings>("save_settings", {
        settings: nextDraft,
        newApiKey: null,
      });
      setDraft(updated);
      onSaved(updated);
      if (showBanner) {
        setStatusMsg({ ok: true, text: s.savedNoTest });
      }
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    }
  };

  const navItems = [
    { id: "model", label: s.secModel, Icon: Cpu },
    { id: "general", label: s.secGeneral, Icon: Sliders },
    { id: "mcp", label: s.secMcp, Icon: Plug },
    { id: "mobile", label: s.secMobile, Icon: Smartphone },
    { id: "actions", label: s.secActions, Icon: Zap },
  ] as const;

  return (
    <div
      className={`flex-1 flex min-h-0 text-white/85 ${
        isWindowMode
          ? "flex-row max-w-5xl w-full mx-auto gap-8 pt-1 text-[13px]"
          : "flex-col text-[12px]"
      }`}
    >
      {/* Navigation: Left vertical rail in Window Mode, top 5-capsule bar in compact Island Mode */}
      {isWindowMode ? (
        <aside className="w-48 shrink-0 border-r border-white/[0.08] pr-4 space-y-1">
          {navItems.map(({ id, label, Icon }) => {
            const active = section === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => setSection(id)}
                className={`w-full px-3 py-2 rounded-xl border text-[13px] font-medium flex items-center gap-2.5 transition ${
                  active
                    ? "bg-white text-black border-white"
                    : "border-transparent text-white/60 hover:text-white hover:bg-white/[0.06]"
                }`}
              >
                <Icon className="w-4 h-4 shrink-0" />
                <span className="truncate">{label}</span>
              </button>
            );
          })}
        </aside>
      ) : (
        <div className="grid grid-cols-5 gap-0.5 p-1 rounded-full bg-white/[0.04] border border-white/10 mb-2 shrink-0">
          {navItems.map(({ id, label }) => {
            const active = section === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => setSection(id)}
                className={`py-1 px-1 rounded-full border text-[10px] font-medium flex items-center justify-center transition-all duration-200 ${
                  active
                    ? "bg-white/[0.12] border-white/[0.16] text-white"
                    : "border-transparent text-white/50 hover:text-white/80 hover:bg-white/[0.04]"
                }`}
              >
                <span className="truncate">{label}</span>
              </button>
            );
          })}
        </div>
      )}

      <div className={`flex-1 flex flex-col min-w-0 min-h-0 ${isWindowMode ? "max-w-2xl" : ""}`}>

      {/* SECTION 1: MULTI-MODEL LIBRARY & ROLE SELECTORS */}
      {section === "model" && (
        <ModelProfilesSection
          settings={settings}
          draft={draft}
          lang={lang}
          statusMsg={statusMsg}
          setStatusMsg={setStatusMsg}
          onUpdateDraft={setDraft}
          onSaved={onSaved}
        />
      )}

      {/* SECTION 2: GENERAL, LANGUAGE & AGENT */}
      {section === "general" && (
        <div
          className={`flex-1 flex flex-col min-h-0 overflow-y-auto pr-0.5 ${
            isWindowMode ? "gap-3.5 pb-6" : "gap-2"
          }`}
        >
          <div
            className={`rounded-2xl bg-white/[0.04] border border-white/10 ${
              isWindowMode ? "p-4 space-y-3" : "p-2.5 space-y-1.5"
            }`}
          >
            {/* System UI Language (English / 中文) */}
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.language}</div>
                <div className="text-[10px] text-white/45 truncate">{s.languageDesc}</div>
              </div>
              <div className="w-28 shrink-0">
                <DarkSelect
                  compact
                  value={lang}
                  options={LANGUAGE_OPTIONS}
                  onChange={(nextLang) => {
                    const next = { ...draft, ui_language: nextLang };
                    setDraft(next);
                    void handleSaveDirect(next, false);
                  }}
                />
              </div>
            </div>

            {/* Thinking Effort (Dynamically adapts to active LLM) */}
            {(() => {
              const activeChatModel = resolveActiveChatModel(draft);
              const thinkingCap = getModelThinkingCapability(activeChatModel);
              const rawThinking = normalizeThinkingLevel(draft.thinking_level);
              const effectiveThinking = clampThinkingLevel(rawThinking, thinkingCap);
              const stepDisplay = getStepDisplay(effectiveThinking, lang, thinkingCap);
              const rawDisplay = getStepDisplay(rawThinking, lang, thinkingCap);
              const badge = formatThinkingBadge(thinkingCap, lang);
              return (
                <div className="pt-1.5 border-t border-white/[0.06] space-y-1">
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0 flex items-center gap-1.5">
                      <div className="text-[11.5px] font-medium text-white shrink-0">
                        {s.thinkingLevel}
                      </div>
                      <span
                        className={`px-1.5 py-[1px] rounded-full text-[9px] border truncate ${
                          thinkingCap.isNative
                            ? "bg-sky-500/15 border-sky-400/25 text-sky-200"
                            : "bg-white/[0.05] border-white/10 text-white/55"
                        }`}
                        title={s.thinkingLevelDesc}
                      >
                        {badge}
                      </span>
                    </div>
                    <ThinkingSlider
                      value={effectiveThinking}
                      capability={thinkingCap}
                      lang={lang}
                      onChange={(lvl) => {
                        const next = { ...draft, thinking_level: lvl };
                        setDraft(next);
                        void handleSaveDirect(next, false);
                      }}
                    />
                  </div>
                  <div className="text-[10px] text-white/45 truncate" title={stepDisplay.desc}>
                    {stepDisplay.desc}
                  </div>
                  {rawThinking !== effectiveThinking && (
                    <div className="text-[9.5px] text-amber-300/80 truncate">
                      {s.thinkingAdapted
                        .replace("{from}", rawDisplay.label)
                        .replace("{to}", stepDisplay.label)}
                    </div>
                  )}
                </div>
              );
            })()}

            {/* Desktop Agent Mode + Workspace */}
            <div className="pt-1.5 border-t border-white/[0.06] space-y-1.5">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-[11.5px] font-medium text-white">{s.agentMode}</div>
                  <div className="text-[10px] text-white/45 truncate">{s.agentModeDesc}</div>
                </div>
                <ToggleSwitch
                  checked={draft.agent_mode}
                  onChange={(checked) => {
                    const next = { ...draft, agent_mode: checked };
                    setDraft(next);
                    void handleSaveDirect(next, false);
                  }}
                />
              </div>
              {draft.agent_mode && (
                <div className="flex items-center gap-1.5">
                  <input
                    type="text"
                    value={draft.agent_workspace}
                    onChange={(e) => setDraft({ ...draft, agent_workspace: e.target.value })}
                    onBlur={() => void handleSaveDirect(draft, false)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.currentTarget.blur();
                      }
                    }}
                    placeholder={s.workspacePlaceholder}
                    className="flex-1 min-w-0 rounded-lg bg-black/30 border border-white/10 focus:border-sky-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white/85 placeholder:text-white/30"
                  />
                  <button
                    type="button"
                    onClick={() => {
                      void invoke<string | null>("pick_folder", {
                        title: s.workspace,
                        initialDir: draft.agent_workspace.trim() || null,
                      }).then((dir) => {
                        if (dir) {
                          const next = { ...draft, agent_workspace: dir };
                          setDraft(next);
                          void handleSaveDirect(next, false);
                        }
                      });
                    }}
                    title={s.browseBtn}
                    className="shrink-0 inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-white/[0.08] hover:bg-white/[0.15] border border-white/10 text-[10.5px] text-white/85 transition"
                  >
                    <FolderOpen className="w-3 h-3 text-sky-300" />
                    <span>{s.browseBtn}</span>
                  </button>
                </div>
              )}
            </div>

            <div className="pt-1.5 border-t border-white/[0.06] flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.musicReactive}</div>
                <div className="text-[10px] text-white/45 truncate">{s.musicReactiveDesc}</div>
              </div>
              <ToggleSwitch
                checked={draft.music_reactive}
                onChange={(checked) => {
                  const next = { ...draft, music_reactive: checked };
                  setDraft(next);
                  void handleSaveDirect(next, false);
                }}
              />
            </div>

            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.autoHide}</div>
                <div className="text-[10px] text-white/45 truncate">{s.autoHideDesc}</div>
              </div>
              <ToggleSwitch
                checked={draft.auto_hide_stealth}
                onChange={(checked) => {
                  const next = { ...draft, auto_hide_stealth: checked };
                  setDraft(next);
                  void handleSaveDirect(next, false);
                }}
              />
            </div>

            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.startup}</div>
                <div className="text-[10px] text-white/45 truncate">{s.startupDesc}</div>
              </div>
              <ToggleSwitch
                checked={draft.launch_at_startup}
                onChange={(checked) => {
                  const next = { ...draft, launch_at_startup: checked };
                  setDraft(next);
                  void handleSaveDirect(next, false);
                }}
              />
            </div>

            <div className="pt-1.5 border-t border-white/[0.06] flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.resetDock}</div>
                <div className="text-[10px] text-white/45 truncate">{s.resetDockDesc}</div>
              </div>
              <button
                type="button"
                onClick={onResetDock}
                className="shrink-0 inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-white/[0.07] hover:bg-white/[0.14] border border-white/10 text-[10.5px] text-white/85 transition"
              >
                <RotateCcw className="w-3 h-3" />
                <span>{s.resetBtn}</span>
              </button>
            </div>

            <div className="pt-1.5 border-t border-white/[0.06] flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.openLogs}</div>
                <div className="text-[10px] text-white/45 truncate">{s.openLogsDesc}</div>
              </div>
              <button
                type="button"
                onClick={() => void invoke("open_logs_folder")}
                className="shrink-0 inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-white/[0.07] hover:bg-white/[0.14] border border-white/10 text-[10.5px] text-white/85 transition"
              >
                <FolderOpen className="w-3 h-3 text-sky-300" />
                <span>{s.openLogsBtn}</span>
              </button>
            </div>

            <div className="pt-1.5 border-t border-white/[0.06] flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.clearBrowser}</div>
                <div className="text-[10px] text-white/45 truncate">{s.clearBrowserDesc}</div>
              </div>
              <button
                type="button"
                onClick={() =>
                  void invoke("clear_browser_data")
                    .then(() => {
                      setBrowserCleared(true);
                      window.setTimeout(() => setBrowserCleared(false), 2000);
                    })
                    .catch(() => {})
                }
                className="shrink-0 inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-white/[0.07] hover:bg-rose-500/25 border border-white/10 text-[10.5px] text-white/85 transition"
              >
                <RotateCcw className="w-3 h-3 text-rose-300" />
                <span>{browserCleared ? s.browserCleared : s.clearBrowserBtn}</span>
              </button>
            </div>
          </div>

          {/* Tauri Official Auto-Updater Card */}
          <div className="rounded-2xl bg-white/[0.04] border border-white/10 p-2.5 space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-white">{s.updatesTitle}</div>
                <div className="text-[10px] text-white/45 font-mono">
                  {s.currentVersion}: v{appVersion}
                </div>
              </div>
              {availableUpdate ? (
                <button
                  type="button"
                  disabled={updateProgress !== null}
                  onClick={onInstallUpdate}
                  className="shrink-0 inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-60 text-[10.5px] text-white font-medium transition"
                >
                  <Download className="w-3 h-3" />
                  <span>
                    {updateProgress !== null
                      ? `${updateProgress}%`
                      : `v${availableUpdate.version}`}
                  </span>
                </button>
              ) : (
                <button
                  type="button"
                  disabled={isCheckingUpdate}
                  onClick={async () => {
                    setIsCheckingUpdate(true);
                    setUpdateMsg(null);
                    const res = await onCheckUpdate();
                    setIsCheckingUpdate(false);
                    if (res.error) {
                      setUpdateMsg({ ok: false, text: res.error });
                    } else if (res.info) {
                      setUpdateMsg({ ok: true, text: `v${res.info.version}` });
                    } else {
                      setUpdateMsg({ ok: true, text: s.upToDate });
                    }
                  }}
                  className="shrink-0 inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-white/[0.08] hover:bg-white/[0.14] border border-white/10 text-[10.5px] text-white transition"
                >
                  {isCheckingUpdate && <Loader2 className="w-3 h-3 animate-spin" />}
                  <span>{isCheckingUpdate ? s.checkingUpdates : s.checkUpdates}</span>
                </button>
              )}
            </div>

            <input
              type="text"
              value={draft.update_endpoint ?? ""}
              onChange={(e) => setDraft({ ...draft, update_endpoint: e.target.value })}
              onBlur={() => void handleSaveDirect(draft, false)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
              }}
              placeholder={s.updateEndpointPlaceholder}
              title={s.updateEndpoint}
              className="w-full rounded-lg bg-black/30 border border-white/10 focus:border-sky-400/50 outline-none px-2 py-1 text-[10px] font-mono text-white/80 placeholder:text-white/30"
            />
            {updateMsg && (
              <div className={`text-[10px] ${updateMsg.ok ? "text-emerald-300" : "text-amber-300/90"}`}>
                {updateMsg.text}
              </div>
            )}
          </div>

          <div className="rounded-2xl bg-white/[0.04] border border-white/10 px-3 py-1.5 flex items-center justify-between text-[10.5px]">
            <span className="text-white/50">{s.displayVsync}</span>
            <span className="font-mono text-emerald-400">
              {refreshHz} Hz · {expandedDims[0]}×{expandedDims[1]}
            </span>
          </div>
        </div>
      )}

      {/* SECTION 3: MCP SERVERS */}
      {section === "mcp" && (
        <div className="flex-1 flex flex-col min-h-0 space-y-2">
          {mcpError && (
            <div className="rounded-xl bg-rose-500/15 border border-rose-400/30 px-2.5 py-1.5 text-[10.5px] text-rose-200 flex items-center justify-between gap-2 shrink-0">
              <span className="truncate">{mcpError}</span>
              <button
                type="button"
                onClick={() => setMcpError(null)}
                className="text-rose-200/70 hover:text-white shrink-0"
              >
                ✕
              </button>
            </div>
          )}
          <McpSection
            lang={lang}
            servers={mcpServers}
            onRefreshServers={setMcpServers}
            onError={setMcpError}
          />
        </div>
      )}

      {/* SECTION 4: MOBILE COMPANION */}
      {section === "mobile" && <MobileLinkCard lang={lang} isWindowMode={isWindowMode} />}

      {/* SECTION 5: QUICK ACTIONS */}
      {section === "actions" && (
        <div
          className={`flex-1 flex flex-col min-h-0 overflow-y-auto ${
            isWindowMode ? "gap-4" : "gap-2.5"
          }`}
        >
          <div className={`grid grid-cols-2 ${isWindowMode ? "gap-3" : "gap-2"}`}>
            <button
              type="button"
              onClick={onTriggerSnip}
              className={`rounded-2xl bg-white/[0.05] hover:bg-white/[0.1] border border-white/10 text-left space-y-1 transition ${
                isWindowMode ? "p-4" : "p-3"
              }`}
            >
              <Camera className="w-4 h-4 text-white/85" />
              <div className="text-[12.5px] font-medium text-white">{s.quickSnip}</div>
              <div className="text-[11px] text-white/45">{s.quickSnipDesc}</div>
            </button>

            <button
              type="button"
              onClick={onTestInhale}
              className={`rounded-2xl bg-white/[0.05] hover:bg-white/[0.1] border border-white/10 text-left space-y-1 transition ${
                isWindowMode ? "p-4" : "p-3"
              }`}
            >
              <Sparkles className="w-4 h-4 text-emerald-400" />
              <div className="text-[12.5px] font-medium text-white">{s.testInhale}</div>
              <div className="text-[11px] text-white/45">{s.testInhaleDesc}</div>
            </button>

            <div
              className={`col-span-2 rounded-2xl bg-white/[0.04] border border-white/10 flex items-center justify-between gap-2 ${
                isWindowMode ? "px-4 py-3" : "px-3 py-2"
              }`}
            >
              <div className="min-w-0">
                <div className="text-[12px] font-medium text-white">{s.toggleHotkey}</div>
                <div className="text-[10.5px] text-white/45">{s.toggleHotkeyDesc}</div>
              </div>
              <kbd className="px-2.5 py-0.5 rounded-md bg-white/[0.08] border border-white/15 font-mono text-[11px] text-white">
                Ctrl + Alt + B
              </kbd>
            </div>
          </div>

          <div className="text-[11px] text-white/40 text-center pt-1">{s.footerNote}</div>
        </div>
      )}
      </div>
    </div>
  );
}
