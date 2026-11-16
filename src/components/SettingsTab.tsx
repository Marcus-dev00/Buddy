import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Camera,
  Check,
  ChevronDown,
  Cpu,
  Download,
  Loader2,
  RotateCcw,
  ShieldCheck,
  Sliders,
  Sparkles,
  Zap,
} from "lucide-react";
import { getStrings } from "../i18n";
import { ThinkingSlider } from "./ChatTab";
import type { AppSettings, ThinkingLevel, UiLang, UpdateInfo } from "../types";

interface SettingsTabProps {
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

type SettingsSection = "model" | "general" | "actions";

interface ModelPreset {
  id: string;
  label: string;
  provider: "openai" | "anthropic" | "gemini";
  baseUrl: string;
  modelId: string;
}

const MODEL_PRESETS: ModelPreset[] = [
  {
    id: "mimo",
    label: "Xiaomi MiMo 2.6 Flash",
    provider: "openai",
    baseUrl: "https://api.xiaomimimo.com/v1",
    modelId: "mimo-v2-flash",
  },
  {
    id: "gemini_flash",
    label: "Gemini 2.5 Flash",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    modelId: "gemini-2.5-flash",
  },
  {
    id: "gemini_pro",
    label: "Gemini 2.5 Pro",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    modelId: "gemini-2.5-pro",
  },
  {
    id: "claude_sonnet",
    label: "Claude Sonnet 4.5",
    provider: "anthropic",
    baseUrl: "https://api.anthropic.com",
    modelId: "claude-sonnet-4-5",
  },
  {
    id: "claude_haiku",
    label: "Claude Haiku 3.5",
    provider: "anthropic",
    baseUrl: "https://api.anthropic.com",
    modelId: "claude-3-5-haiku-latest",
  },
  {
    id: "openai_4o_mini",
    label: "OpenAI GPT-4o Mini",
    provider: "openai",
    baseUrl: "https://api.openai.com/v1",
    modelId: "gpt-4o-mini",
  },
  {
    id: "openai_41_mini",
    label: "OpenAI GPT-4.1 Mini",
    provider: "openai",
    baseUrl: "https://api.openai.com/v1",
    modelId: "gpt-4.1-mini",
  },
  {
    id: "deepseek",
    label: "DeepSeek Chat (V3)",
    provider: "openai",
    baseUrl: "https://api.deepseek.com/v1",
    modelId: "deepseek-chat",
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    provider: "openai",
    baseUrl: "https://openrouter.ai/api/v1",
    modelId: "openai/gpt-4o-mini",
  },
  {
    id: "ollama",
    label: "Ollama (Local)",
    provider: "openai",
    baseUrl: "http://localhost:11434/v1",
    modelId: "llama3.2",
  },
  {
    id: "custom",
    label: "Custom Endpoint",
    provider: "openai",
    baseUrl: "",
    modelId: "",
  },
];

const PROTOCOL_OPTIONS = [
  { id: "openai", label: "OpenAI Compatible" },
  { id: "anthropic", label: "Anthropic Claude" },
  { id: "gemini", label: "Google Gemini" },
];

const LANGUAGE_OPTIONS = [
  { id: "en", label: "English" },
  { id: "zh", label: "简体中文" },
];

interface DarkSelectProps {
  value: string;
  options: { id: string; label: string }[];
  onChange: (id: string) => void;
  onOpen?: () => void;
  compact?: boolean;
}

function DarkSelect({ value, options, onChange, onOpen, compact }: DarkSelectProps) {
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const current = options.find((o) => o.id === value) ?? options[0];

  return (
    <div ref={boxRef} className="relative">
      <button
        type="button"
        onClick={() => {
          if (!open && onOpen) onOpen();
          setOpen((v) => !v);
        }}
        className={`w-full rounded-lg bg-white/[0.07] hover:bg-white/[0.11] border border-white/15 text-left text-[11px] text-white flex items-center justify-between gap-1.5 transition ${
          compact ? "px-2 py-1" : "px-2.5 py-1.5"
        }`}
      >
        <span className="truncate">{current?.label ?? value}</span>
        <ChevronDown
          className={`w-3 h-3 text-white/50 shrink-0 transition-transform ${
            open ? "rotate-180" : ""
          }`}
        />
      </button>

      {open && (
        <div className="absolute left-0 right-0 mt-1 z-50 max-h-40 overflow-y-auto rounded-xl bg-[#0e0f14] border border-white/15 shadow-2xl p-1">
          {options.map((opt) => {
            const active = opt.id === value;
            return (
              <button
                key={opt.id}
                type="button"
                onClick={() => {
                  onChange(opt.id);
                  setOpen(false);
                }}
                className={`w-full text-left px-2 py-1.5 rounded-lg text-[11px] flex items-center justify-between transition ${
                  active
                    ? "bg-sky-500/20 text-sky-200 font-medium"
                    : "text-white/80 hover:bg-white/[0.08]"
                }`}
              >
                <span className="truncate">{opt.label}</span>
                {active && <Check className="w-3 h-3 text-sky-400 shrink-0" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ToggleSwitch({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`w-8 h-[18px] rounded-full p-0.5 transition-colors flex items-center shrink-0 ${
        checked ? "bg-sky-500" : "bg-white/15"
      }`}
    >
      <span
        className={`w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
          checked ? "translate-x-[14px]" : "translate-x-0"
        }`}
      />
    </button>
  );
}

function detectPresetId(cfg: AppSettings): string {
  const b = cfg.base_url.trim().replace(/\/+$/, "");
  const m = cfg.model_id.trim();
  const match = MODEL_PRESETS.find(
    (p) => p.id !== "custom" && p.provider === cfg.api_provider && p.baseUrl === b && p.modelId === m
  );
  return match?.id ?? "custom";
}

export default function SettingsTab({
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
  const [presetId, setPresetId] = useState<string>(() => detectPresetId(settings));
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [keyDirty, setKeyDirty] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [statusMsg, setStatusMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [updateMsg, setUpdateMsg] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  const lang: UiLang = draft.ui_language === "zh" ? "zh" : "en";
  const s = getStrings(lang).settings;

  const handlePresetChange = (id: string) => {
    setPresetId(id);
    setStatusMsg(null);
    const preset = MODEL_PRESETS.find((p) => p.id === id);
    if (preset && preset.id !== "custom") {
      setDraft((d) => ({
        ...d,
        api_provider: preset.provider,
        base_url: preset.baseUrl,
        model_id: preset.modelId,
      }));
    }
  };

  const handleVerifyAndSave = async () => {
    setIsTesting(true);
    setStatusMsg(null);
    try {
      const res = await invoke<{ settings: AppSettings; latency_ms: number }>(
        "verify_and_save_model_config",
        {
          settings: draft,
          newApiKey: keyDirty ? apiKeyInput : null,
        }
      );
      setDraft(res.settings);
      setKeyDirty(false);
      setApiKeyInput("");
      onSaved(res.settings);
      setStatusMsg({ ok: true, text: `${s.connectedSaved} (${res.latency_ms}ms)` });
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    } finally {
      setIsTesting(false);
    }
  };

  const handleSaveDirect = async (nextDraft = draft, showBanner = true) => {
    if (showBanner) setStatusMsg(null);
    try {
      const updated = await invoke<AppSettings>("save_settings", {
        settings: nextDraft,
        newApiKey: keyDirty ? apiKeyInput : null,
      });
      setDraft(updated);
      setKeyDirty(false);
      setApiKeyInput("");
      onSaved(updated);
      if (showBanner) {
        setStatusMsg({ ok: true, text: s.savedNoTest });
      }
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 text-[12px] text-white/85">
      {/* Sub-navigation Pill Switcher */}
      <div className="grid grid-cols-3 gap-1 p-1 rounded-xl bg-white/[0.05] border border-white/10 mb-2 shrink-0">
        {(
          [
            { id: "model", label: s.secModel, Icon: Cpu },
            { id: "general", label: s.secGeneral, Icon: Sliders },
            { id: "actions", label: s.secActions, Icon: Zap },
          ] as const
        ).map(({ id, label, Icon }) => {
          const active = section === id;
          return (
            <button
              key={id}
              type="button"
              onClick={() => setSection(id)}
              className={`py-1 rounded-lg text-[11px] font-medium flex items-center justify-center gap-1.5 transition ${
                active
                  ? "bg-white/15 text-white"
                  : "text-white/50 hover:text-white/80"
              }`}
            >
              <Icon className="w-3 h-3" />
              <span>{label}</span>
            </button>
          );
        })}
      </div>

      {/* SECTION 1: MODEL & PRESETS */}
      {section === "model" && (
        <div className="flex-1 flex flex-col justify-between min-h-0">
          <div className="space-y-2">
            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1">
                <label className="text-[10.5px] text-white/50">{s.preset}</label>
                <DarkSelect
                  value={presetId}
                  options={MODEL_PRESETS.map((p) => ({ id: p.id, label: p.label }))}
                  onChange={handlePresetChange}
                />
              </div>
              <div className="space-y-1">
                <label className="text-[10.5px] text-white/50">{s.protocol}</label>
                <DarkSelect
                  value={draft.api_provider}
                  options={PROTOCOL_OPTIONS}
                  onChange={(prov) => {
                    setPresetId("custom");
                    setDraft({ ...draft, api_provider: prov });
                  }}
                />
              </div>
            </div>

            <div className="space-y-1">
              <label className="text-[10.5px] text-white/50">{s.baseUrl}</label>
              <input
                type="text"
                value={draft.base_url}
                onChange={(e) => {
                  setPresetId("custom");
                  setDraft({ ...draft, base_url: e.target.value });
                }}
                placeholder="https://..."
                className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2.5 py-1 text-white font-mono text-[11px]"
              />
            </div>

            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1">
                <label className="text-[10.5px] text-white/50">{s.modelId}</label>
                <input
                  type="text"
                  value={draft.model_id}
                  onChange={(e) => {
                    setPresetId("custom");
                    setDraft({ ...draft, model_id: e.target.value });
                  }}
                  className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2.5 py-1 text-white font-mono text-[11px]"
                />
              </div>
              <div className="space-y-1">
                <div className="flex items-center justify-between">
                  <label className="text-[10.5px] text-white/50">{s.apiKey}</label>
                  {draft.has_api_key && (
                    <span className="flex items-center gap-0.5 text-[10px] text-emerald-400">
                      <ShieldCheck className="w-2.5 h-2.5" />
                      DPAPI
                    </span>
                  )}
                </div>
                <input
                  type="password"
                  value={apiKeyInput}
                  onChange={(e) => {
                    setApiKeyInput(e.target.value);
                    setKeyDirty(true);
                  }}
                  placeholder={
                    draft.has_api_key
                      ? "••••••••••••"
                      : draft.api_provider === "anthropic"
                      ? "sk-ant-..."
                      : draft.api_provider === "gemini"
                      ? "AIza..."
                      : "sk-..."
                  }
                  className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2.5 py-1 text-white font-mono text-[11px] placeholder:text-white/30"
                />
              </div>
            </div>
          </div>

          {/* Connection Probe Footer */}
          <div className="pt-2 border-t border-white/[0.08] flex items-center justify-between gap-2 shrink-0">
            <div className="min-w-0 flex-1">
              {statusMsg ? (
                <div className="flex items-center gap-1.5">
                  <span
                    className={`text-[10.5px] truncate ${
                      statusMsg.ok ? "text-emerald-400" : "text-rose-400"
                    }`}
                    title={statusMsg.text}
                  >
                    {statusMsg.text}
                  </span>
                  {!statusMsg.ok && (
                    <button
                      type="button"
                      onClick={() => void handleSaveDirect(draft)}
                      className="text-[10px] underline text-white/60 hover:text-white shrink-0"
                    >
                      {s.saveAnyway}
                    </button>
                  )}
                </div>
              ) : (
                <span className="text-[10.5px] text-white/40">{s.verifyHint}</span>
              )}
            </div>

            <button
              type="button"
              disabled={isTesting}
              onClick={() => void handleVerifyAndSave()}
              className="px-3 py-1.5 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-60 text-slate-950 font-semibold text-[11px] flex items-center gap-1.5 shrink-0 transition"
            >
              {isTesting ? (
                <>
                  <Loader2 className="w-3 h-3 animate-spin" />
                  <span>{s.testing}</span>
                </>
              ) : (
                <span>{s.testAndSave}</span>
              )}
            </button>
          </div>
        </div>
      )}

      {/* SECTION 2: GENERAL, LANGUAGE & AGENT */}
      {section === "general" && (
        <div className="flex-1 flex flex-col justify-between min-h-0 gap-1.5 overflow-y-auto pr-0.5">
          <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2.5 space-y-1.5">
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

            {/* Thinking Effort (Horizontal Slider: Off -> Max) */}
            <div className="pt-1.5 border-t border-white/[0.06] space-y-1">
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="text-[11.5px] font-medium text-white">{s.thinkingLevel}</div>
                </div>
                <ThinkingSlider
                  value={
                    (draft.thinking_level === "off" ||
                    draft.thinking_level === "low" ||
                    draft.thinking_level === "high" ||
                    draft.thinking_level === "max"
                      ? draft.thinking_level
                      : "medium") as ThinkingLevel
                  }
                  lang={lang}
                  onChange={(lvl) => {
                    const next = { ...draft, thinking_level: lvl };
                    setDraft(next);
                    void handleSaveDirect(next, false);
                  }}
                />
              </div>
              <div className="text-[10px] text-white/45 truncate">
                {
                  getStrings(lang).chat.thinkLevels[
                    (draft.thinking_level === "off" ||
                    draft.thinking_level === "low" ||
                    draft.thinking_level === "high" ||
                    draft.thinking_level === "max"
                      ? draft.thinking_level
                      : "medium") as ThinkingLevel
                  ].desc
                }
              </div>
            </div>

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
                  className="w-full rounded-lg bg-black/30 border border-white/10 focus:border-sky-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white/85 placeholder:text-white/30"
                />
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
          </div>

          {/* Tauri Official Auto-Updater Card */}
          <div className="rounded-xl bg-white/[0.04] border border-white/10 p-2.5 space-y-1.5">
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

          <div className="rounded-xl bg-white/[0.04] border border-white/10 px-3 py-1.5 flex items-center justify-between text-[10.5px]">
            <span className="text-white/50">{s.displayVsync}</span>
            <span className="font-mono text-emerald-400">
              {refreshHz} Hz · {expandedDims[0]}×{expandedDims[1]}
            </span>
          </div>
        </div>
      )}

      {/* SECTION 3: QUICK ACTIONS */}
      {section === "actions" && (
        <div className="flex-1 flex flex-col justify-between min-h-0">
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={onTriggerSnip}
              className="rounded-xl bg-white/[0.05] hover:bg-sky-500/20 border border-white/10 p-3 text-left space-y-1 transition"
            >
              <Camera className="w-4 h-4 text-sky-400" />
              <div className="text-[12px] font-medium text-white">{s.quickSnip}</div>
              <div className="text-[10.5px] text-white/45">{s.quickSnipDesc}</div>
            </button>

            <button
              type="button"
              onClick={onTestInhale}
              className="rounded-xl bg-white/[0.05] hover:bg-emerald-500/20 border border-white/10 p-3 text-left space-y-1 transition"
            >
              <Sparkles className="w-4 h-4 text-emerald-400" />
              <div className="text-[12px] font-medium text-white">{s.testInhale}</div>
              <div className="text-[10.5px] text-white/45">{s.testInhaleDesc}</div>
            </button>
          </div>

          <div className="text-[10.5px] text-white/40 text-center pb-0.5">{s.footerNote}</div>
        </div>
      )}
    </div>
  );
}
