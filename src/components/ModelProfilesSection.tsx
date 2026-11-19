import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff, Loader2, MessageSquare, Plus, ShieldCheck, Trash2 } from "lucide-react";
import { getStrings } from "../i18n";
import { normalizeThinkingMode } from "../thinking";
import type { AppSettings, ModelProfile, UiLang } from "../types";
import { DarkSelect, PROTOCOL_OPTIONS } from "./SettingsControls";

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
    id: "gemini_35_flash_lite",
    label: "Gemini 3.5 Flash Lite",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    modelId: "gemini-3.5-flash-lite",
  },
  {
    id: "gemini_35_flash",
    label: "Gemini 3.5 Flash",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    modelId: "gemini-3.5-flash",
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

function detectPresetId(cfg: { api_provider: string; base_url: string; model_id: string }): string {
  const b = cfg.base_url.trim().replace(/\/+$/, "");
  const m = cfg.model_id.trim();
  const match = MODEL_PRESETS.find(
    (p) => p.id !== "custom" && p.provider === cfg.api_provider && p.baseUrl === b && p.modelId === m
  );
  return match?.id ?? "custom";
}

function makeFallbackProfiles(cfg: AppSettings): ModelProfile[] {
  if (cfg.model_profiles && cfg.model_profiles.length > 0) {
    return cfg.model_profiles;
  }
  return [
    {
      id: cfg.chat_profile_id || "prof_default",
      name: cfg.model_id || "Default Model",
      api_provider: cfg.api_provider,
      base_url: cfg.base_url,
      model_id: cfg.model_id,
      supports_vision: true,
      thinking_mode: "auto",
      has_api_key: cfg.has_api_key,
    },
  ];
}

export interface ModelProfilesSectionProps {
  settings: AppSettings;
  draft: AppSettings;
  lang: UiLang;
  statusMsg: { ok: boolean; text: string } | null;
  setStatusMsg: (next: { ok: boolean; text: string } | null) => void;
  onUpdateDraft: (updated: AppSettings) => void;
  onSaved: (updated: AppSettings) => void;
}

export function ModelProfilesSection({
  settings,
  draft,
  lang,
  statusMsg,
  setStatusMsg,
  onUpdateDraft,
  onSaved,
}: ModelProfilesSectionProps) {
  const initialProfiles = makeFallbackProfiles(settings);
  const [editingProfile, setEditingProfile] = useState<ModelProfile>(
    () => initialProfiles.find((p) => p.id === settings.chat_profile_id) ?? initialProfiles[0]
  );
  const [isNewProfile, setIsNewProfile] = useState(false);
  const [presetId, setPresetId] = useState<string>(() => detectPresetId(editingProfile));
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [keyDirty, setKeyDirty] = useState(false);
  const [isTesting, setIsTesting] = useState(false);

  const prevProfilesSyncRef = useRef(
    JSON.stringify({
      profiles: settings.model_profiles ?? [],
      chat: settings.chat_profile_id ?? "",
      api_provider: settings.api_provider,
      base_url: settings.base_url,
      model_id: settings.model_id,
      has_api_key: settings.has_api_key,
    })
  );

  useEffect(() => {
    const nextSig = JSON.stringify({
      profiles: settings.model_profiles ?? [],
      chat: settings.chat_profile_id ?? "",
      api_provider: settings.api_provider,
      base_url: settings.base_url,
      model_id: settings.model_id,
      has_api_key: settings.has_api_key,
    });
    if (nextSig === prevProfilesSyncRef.current) return;
    prevProfilesSyncRef.current = nextSig;
    const profs = makeFallbackProfiles(settings);
    setEditingProfile((cur) => {
      if (isNewProfile) return cur;
      return (
        profs.find((p) => p.id === cur.id) ??
        profs.find((p) => p.id === settings.chat_profile_id) ??
        profs[0]
      );
    });
  }, [settings, isNewProfile]);

  const s = getStrings(lang).settings;
  const profiles = makeFallbackProfiles(draft);
  const activeChatId = draft.chat_profile_id || profiles[0]?.id || "prof_default";
  const activeWatchId = draft.watch_profile_id || "";

  const selectProfileToEdit = (prof: ModelProfile) => {
    setIsNewProfile(false);
    setEditingProfile(prof);
    setPresetId(detectPresetId(prof));
    setApiKeyInput("");
    setKeyDirty(false);
    setStatusMsg(null);
  };

  const handleStartNewProfile = () => {
    const newId = `prof_${Date.now().toString(36)}`;
    const blank: ModelProfile = {
      id: newId,
      name: "Gemini 2.5 Flash",
      api_provider: "gemini",
      base_url: "https://generativelanguage.googleapis.com",
      model_id: "gemini-2.5-flash",
      supports_vision: true,
      thinking_mode: "auto",
      has_api_key: false,
    };
    setIsNewProfile(true);
    setEditingProfile(blank);
    setPresetId("gemini_flash");
    setApiKeyInput("");
    setKeyDirty(false);
    setStatusMsg(null);
  };

  const handlePresetChange = (id: string) => {
    setPresetId(id);
    setStatusMsg(null);
    const preset = MODEL_PRESETS.find((p) => p.id === id);
    if (preset && preset.id !== "custom") {
      setEditingProfile((p) => ({
        ...p,
        name: preset.label,
        api_provider: preset.provider,
        base_url: preset.baseUrl,
        model_id: preset.modelId,
        supports_vision: preset.id === "deepseek" ? false : true,
      }));
    }
  };

  const handleRoleChange = async (nextChatId: string | null, nextWatchId: string | null) => {
    setStatusMsg(null);
    try {
      const updated = await invoke<AppSettings>("select_role_models", {
        chatProfileId: nextChatId,
        watchProfileId: nextWatchId,
      });
      onUpdateDraft(updated);
      onSaved(updated);
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    }
  };

  const handleVerifyAndSaveProfile = async () => {
    setIsTesting(true);
    setStatusMsg(null);
    try {
      const res = await invoke<{ settings: AppSettings; latency_ms: number }>(
        "verify_and_save_model_profile",
        {
          profile: editingProfile,
          newApiKey: keyDirty ? apiKeyInput : null,
          setAsChat: isNewProfile ? false : editingProfile.id === activeChatId,
          setAsWatch: false,
        }
      );
      setIsNewProfile(false);
      onUpdateDraft(res.settings);
      const savedProf =
        makeFallbackProfiles(res.settings).find((p) => p.id === editingProfile.id) ??
        editingProfile;
      setEditingProfile(savedProf);
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

  const handleSaveProfileDirect = async () => {
    setStatusMsg(null);
    try {
      const updated = await invoke<AppSettings>("save_model_profile", {
        profile: editingProfile,
        newApiKey: keyDirty ? apiKeyInput : null,
        setAsChat: isNewProfile ? false : editingProfile.id === activeChatId,
        setAsWatch: false,
      });
      setIsNewProfile(false);
      onUpdateDraft(updated);
      const savedProf =
        makeFallbackProfiles(updated).find((p) => p.id === editingProfile.id) ?? editingProfile;
      setEditingProfile(savedProf);
      setKeyDirty(false);
      setApiKeyInput("");
      onSaved(updated);
      setStatusMsg({ ok: true, text: s.savedNoTest });
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    }
  };

  const handleDeleteProfile = async (profileId: string) => {
    setStatusMsg(null);
    try {
      const updated = await invoke<AppSettings>("delete_model_profile", { profileId });
      setIsNewProfile(false);
      onUpdateDraft(updated);
      const profs = makeFallbackProfiles(updated);
      setEditingProfile(profs.find((p) => p.id === updated.chat_profile_id) ?? profs[0]);
      onSaved(updated);
    } catch (err) {
      setStatusMsg({ ok: false, text: String(err) });
    }
  };

  return (
    <div className="flex-1 flex flex-col justify-between min-h-0 gap-1.5">
      <div className="flex-1 overflow-y-auto pr-0.5 space-y-2 min-h-0">
        {/* Chat / Watch Role Selectors */}
        <div className="rounded-2xl bg-white/[0.04] border border-white/10 p-2.5 grid grid-cols-2 gap-2">
          <div className="space-y-1">
            <label className="text-[10px] text-white/55 flex items-center gap-1">
              <MessageSquare className="w-2.5 h-2.5 text-sky-300" />
              <span className="truncate">{s.roleChatModel}</span>
            </label>
            <DarkSelect
              compact
              value={activeChatId}
              options={profiles.map((p) => ({
                id: p.id,
                label: `${p.name || p.model_id}`,
              }))}
              onChange={(id) => void handleRoleChange(id, null)}
            />
          </div>

          <div className="space-y-1">
            <label className="text-[10px] text-white/55 flex items-center gap-1">
              <Eye className="w-2.5 h-2.5 text-rose-300" />
              <span className="truncate">{s.roleWatchModel}</span>
            </label>
            <DarkSelect
              compact
              value={activeWatchId || "follow_chat"}
              options={[
                { id: "follow_chat", label: s.followChat },
                ...profiles.map((p) => ({
                  id: p.id,
                  label: `${p.name || p.model_id}${p.supports_vision ? "" : " (Text)"}`,
                })),
              ]}
              onChange={(id) => void handleRoleChange(null, id)}
            />
          </div>
        </div>

        {/* Saved Profiles Chips */}
        <div className="space-y-1">
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-white/50">{s.modelLibrary}</span>
            <button
              type="button"
              onClick={handleStartNewProfile}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-white/[0.08] hover:bg-white/[0.15] border border-white/10 text-[10px] text-white/85 transition"
            >
              <Plus className="w-2.5 h-2.5" />
              <span>{s.newProfile}</span>
            </button>
          </div>
          <div className="flex flex-wrap gap-1">
            {profiles.map((prof) => {
              const selected = !isNewProfile && editingProfile.id === prof.id;
              const isChat = prof.id === activeChatId;
              const isWatch = prof.id === activeWatchId;
              return (
                <button
                  key={prof.id}
                  type="button"
                  onClick={() => selectProfileToEdit(prof)}
                  className={`px-2 py-0.5 rounded-full text-[10px] border flex items-center gap-1 transition max-w-[170px] ${
                    selected
                      ? "bg-white/[0.13] border-white/30 text-white font-medium shadow-[inset_0_1px_0_rgba(255,255,255,0.16)]"
                      : "bg-white/[0.04] border-white/10 text-white/65 hover:text-white/90"
                  }`}
                >
                  <span className="truncate">{prof.name || prof.model_id}</span>
                  {isChat && (
                    <span className="text-[8.5px] px-1 rounded bg-sky-500/25 text-sky-200 shrink-0">
                      Chat
                    </span>
                  )}
                  {isWatch && (
                    <span className="text-[8.5px] px-1 rounded bg-rose-500/25 text-rose-200 shrink-0">
                      Watch
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        </div>

        {/* Profile Editor */}
        <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-2.5 space-y-1.5">
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-0.5">
              <label className="text-[10px] text-white/50">{s.profileName}</label>
              <input
                type="text"
                value={editingProfile.name}
                onChange={(e) =>
                  setEditingProfile({ ...editingProfile, name: e.target.value })
                }
                placeholder="My Gemini Flash"
                className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2 py-1 text-white text-[11px]"
              />
            </div>
            <div className="space-y-0.5">
              <label className="text-[10px] text-white/50">{s.preset}</label>
              <DarkSelect
                compact
                value={presetId}
                options={MODEL_PRESETS.map((p) => ({ id: p.id, label: p.label }))}
                onChange={handlePresetChange}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-0.5">
              <label className="text-[10px] text-white/50">{s.protocol}</label>
              <DarkSelect
                compact
                value={editingProfile.api_provider}
                options={PROTOCOL_OPTIONS}
                onChange={(prov) => {
                  setPresetId("custom");
                  setEditingProfile({ ...editingProfile, api_provider: prov });
                }}
              />
            </div>
            <div className="space-y-0.5">
              <label className="text-[10px] text-white/50">{s.modelId}</label>
              <input
                type="text"
                value={editingProfile.model_id}
                onChange={(e) => {
                  setPresetId("custom");
                  setEditingProfile({ ...editingProfile, model_id: e.target.value });
                }}
                className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2 py-1 text-white font-mono text-[11px]"
              />
            </div>
          </div>

          <div className="space-y-0.5">
            <label className="text-[10px] text-white/50">{s.baseUrl}</label>
            <input
              type="text"
              value={editingProfile.base_url}
              onChange={(e) => {
                setPresetId("custom");
                setEditingProfile({ ...editingProfile, base_url: e.target.value });
              }}
              placeholder="https://..."
              className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2 py-1 text-white font-mono text-[11px]"
            />
          </div>

          <div className="grid grid-cols-1 gap-1.5">
            <div className="space-y-0.5">
              <div className="flex items-center justify-between">
                <label className="text-[10px] text-white/50">{s.apiKey}</label>
                <div className="flex items-center gap-2">
                  {editingProfile.has_api_key && (
                    <span className="flex items-center gap-0.5 text-[9.5px] text-emerald-400">
                      <ShieldCheck className="w-2.5 h-2.5" />
                      DPAPI
                    </span>
                  )}
                  {!isNewProfile && profiles.length > 1 && (
                    <button
                      type="button"
                      onClick={() => void handleDeleteProfile(editingProfile.id)}
                      className="text-[9.5px] text-rose-300/75 hover:text-rose-300 inline-flex items-center gap-0.5"
                    >
                      <Trash2 className="w-2.5 h-2.5" />
                      <span>{s.deleteProfile}</span>
                    </button>
                  )}
                </div>
              </div>
              <input
                type="password"
                value={apiKeyInput}
                onChange={(e) => {
                  setApiKeyInput(e.target.value);
                  setKeyDirty(true);
                }}
                placeholder={
                  editingProfile.has_api_key
                    ? "••••••••••••"
                    : editingProfile.api_provider === "anthropic"
                    ? "sk-ant-..."
                    : editingProfile.api_provider === "gemini"
                    ? "AIza..."
                    : "sk-..."
                }
                className="w-full rounded-lg bg-white/[0.07] border border-white/15 focus:border-sky-400/60 outline-none px-2 py-1 text-white font-mono text-[11px] placeholder:text-white/30"
              />
            </div>

            <div className="grid grid-cols-2 gap-2 pt-0.5 items-center">
              <label className="flex items-center justify-between gap-2 cursor-pointer select-none">
                <span className="text-[10.5px] text-white/70 flex items-center gap-1.5 truncate">
                  {editingProfile.supports_vision ? (
                    <Eye className="w-3 h-3 text-sky-300 shrink-0" />
                  ) : (
                    <EyeOff className="w-3 h-3 text-white/35 shrink-0" />
                  )}
                  <span className="truncate">{s.supportsVision}</span>
                </span>
                <input
                  type="checkbox"
                  checked={editingProfile.supports_vision}
                  onChange={(e) =>
                    setEditingProfile({
                      ...editingProfile,
                      supports_vision: e.target.checked,
                    })
                  }
                  className="accent-sky-400 w-3.5 h-3.5 rounded shrink-0"
                />
              </label>
              <div className="flex items-center gap-1.5 min-w-0">
                <span className="text-[10px] text-white/50 shrink-0">
                  {s.thinkingModeLabel}
                </span>
                <div className="flex-1 min-w-0">
                  <DarkSelect
                    compact
                    value={normalizeThinkingMode(editingProfile.thinking_mode)}
                    options={[
                      { id: "auto", label: s.thinkingModeAuto },
                      { id: "native", label: s.thinkingModeNative },
                      { id: "prompt", label: s.thinkingModePrompt },
                    ]}
                    onChange={(mode) =>
                      setEditingProfile({ ...editingProfile, thinking_mode: mode })
                    }
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Connection Probe Footer */}
      <div className="pt-1.5 border-t border-white/[0.08] flex items-center justify-between gap-2 shrink-0">
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
                  onClick={() => void handleSaveProfileDirect()}
                  className="text-[10px] underline text-white/60 hover:text-white shrink-0"
                >
                  {s.saveAnyway}
                </button>
              )}
            </div>
          ) : (
            <span className="text-[10px] text-white/40">{s.verifyHint}</span>
          )}
        </div>

        <button
          type="button"
          disabled={isTesting}
          onClick={() => void handleVerifyAndSaveProfile()}
          className="px-3 py-1 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-60 text-slate-950 font-semibold text-[11px] flex items-center gap-1.5 shrink-0 transition"
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
  );
}
