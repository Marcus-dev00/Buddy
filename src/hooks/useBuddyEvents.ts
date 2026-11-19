import { useEffect, useRef } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { FloatingBubble } from "../components/FloatingBubbleOverlay";
import { getStrings } from "../i18n";
import type {
  AgentStep,
  AppSettings,
  AttachmentPayload,
  BrowserPreviewPayload,
  BrowserStatePayload,
  BuddyState,
  ChatMessage,
  ColorPickedPayload,
  DockEdge,
  DockStatePayload,
  LayoutDims,
  MemoryItem,
  MusicPayload,
  PluginListResponse,
  PluginProgressPayload,
  ShareStatePayload,
  ShareWhisperPayload,
  SkillInfo,
  TabId,
  TimerTickPayload,
  UiLang,
  UpdateInfo,
} from "../types";
import {
  headTargetDeg,
  isSideDock,
  type BuddyMusicRuntime,
  type EyePreset,
  type TypewriterRuntime,
} from "./useBuddyMotion";

export function formatBubblePreview(raw: string, fallback: string, maxChars = 88): string {
  const cleaned = raw
    .replace(/```[a-zA-Z0-9_-]*\s*/g, "")
    .replace(/```/g, "")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/[*_`~]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return fallback;
  const chars = Array.from(cleaned);
  if (chars.length <= maxChars) return cleaned;
  return `${chars.slice(0, maxChars).join("")}…`;
}

interface PeekPayload {
  hovering: boolean;
  dx: number;
  dy: number;
}

interface StealthPayload {
  active: boolean;
  reason: string;
}

interface SleepPayload {
  sleeping: boolean;
}

export interface UseBuddyEventsParams {
  stateRef: React.MutableRefObject<BuddyState>;
  dockEdgeRef: React.MutableRefObject<DockEdge>;
  snapPreviewRef: React.MutableRefObject<DockEdge | null>;
  layoutDimsRef: React.MutableRefObject<LayoutDims>;
  activeTabRef: React.MutableRefObject<TabId>;
  peekTargetRef: React.MutableRefObject<{ dx: number; dy: number }>;
  musicRef: React.MutableRefObject<BuddyMusicRuntime>;
  recentTimerRef: React.MutableRefObject<number | null>;
  activeStreamIdRef: React.MutableRefObject<string | null>;
  typewriterRef: React.MutableRefObject<TypewriterRuntime>;
  thinkingRef: React.MutableRefObject<boolean>;
  langRef: React.MutableRefObject<UiLang>;
  shareStateRef: React.MutableRefObject<ShareStatePayload | null>;
  activeConversationIdRef: React.MutableRefObject<number | null>;
  floatingBubbleRef: React.MutableRefObject<FloatingBubble | null>;
  springRef: React.MutableRefObject<{
    w: number;
    h: number;
    capY: number;
    vCapY: number;
    vCapScale: number;
    headRot: number;
    tHeadRot: number;
  }>;
  setLayoutDims: React.Dispatch<React.SetStateAction<LayoutDims>>;
  setDockEdge: React.Dispatch<React.SetStateAction<DockEdge>>;
  setSnapPreview: React.Dispatch<React.SetStateAction<DockEdge | null>>;
  setAppVersion: React.Dispatch<React.SetStateAction<string>>;
  setPluginsData: React.Dispatch<React.SetStateAction<PluginListResponse>>;
  setSkills: React.Dispatch<React.SetStateAction<SkillInfo[]>>;
  setShareState: React.Dispatch<React.SetStateAction<ShareStatePayload | null>>;
  setBrowserState: React.Dispatch<React.SetStateAction<BrowserStatePayload | null>>;
  setBrowserPreview: React.Dispatch<React.SetStateAction<BrowserPreviewPayload | null>>;
  setSettings: React.Dispatch<React.SetStateAction<AppSettings>>;
  setMemoryRefresh: React.Dispatch<React.SetStateAction<number>>;
  setRecentMemory: React.Dispatch<React.SetStateAction<MemoryItem | null>>;
  setAttachments: React.Dispatch<React.SetStateAction<AttachmentPayload[]>>;
  setDroppedPathsHint: React.Dispatch<React.SetStateAction<string[]>>;
  setActiveTab: React.Dispatch<React.SetStateAction<TabId>>;
  setMessages: React.Dispatch<React.SetStateAction<ChatMessage[]>>;
  setIsThinking: React.Dispatch<React.SetStateAction<boolean>>;
  setActiveStreamId: React.Dispatch<React.SetStateAction<string | null>>;
  setAvailableUpdate: React.Dispatch<React.SetStateAction<UpdateInfo | null>>;
  setUpdateProgress: React.Dispatch<React.SetStateAction<number | null>>;
  setPluginProgress: React.Dispatch<React.SetStateAction<Record<string, PluginProgressPayload>>>;
  setLastPickedColor: React.Dispatch<React.SetStateAction<ColorPickedPayload | null>>;
  setFocusedPluginId: React.Dispatch<React.SetStateAction<string | null>>;
  setTimerState: React.Dispatch<React.SetStateAction<TimerTickPayload | null>>;
  loadConversationMessages: (convId: number, langOverride?: UiLang) => Promise<void>;
  refreshConversations: () => Promise<void>;
  applyVisualTargets: (next: BuddyState, thinkingOverride?: boolean) => void;
  transitionTo: (next: BuddyState) => void;
  restStateOrMusic: () => BuddyState;
  triggerCapture: () => void;
  renderEyeSvg: (type: EyePreset, speed?: number) => void;
  presentBubble: (next: FloatingBubble) => void;
  dismissBubbleAnimated: () => void;
}

/**
 * Bootstraps initial persisted app state and wires all Tauri backend event listeners
 * (sensor updates, AI token/thought/step streams, tray/hotkey commands, plugins, screen share).
 */
export function useBuddyEvents(params: UseBuddyEventsParams): void {
  const paramsRef = useRef(params);
  paramsRef.current = params;

  useEffect(() => {
    const p0 = paramsRef.current;

    void (async () => {
      try {
        const dims = await invoke<LayoutDims>("get_expanded_bounds");
        p0.layoutDimsRef.current = dims;
        p0.setLayoutDims(dims);
      } catch {
        // ignore
      }
      try {
        const dock = await invoke<DockStatePayload>("get_dock_state");
        const edge = (dock.dock_edge || "top") as DockEdge;
        p0.dockEdgeRef.current = edge;
        p0.setDockEdge(edge);
        const deg = headTargetDeg(edge);
        p0.springRef.current.headRot = deg;
        p0.springRef.current.tHeadRot = deg;
        const side = isSideDock(edge);
        p0.springRef.current.w = side ? 42 : 152;
        p0.springRef.current.h = side ? 152 : 42;
      } catch {
        // ignore
      }
      try {
        const ver = await invoke<string>("get_app_version");
        p0.setAppVersion(ver);
      } catch {
        // ignore
      }
      try {
        const pl = await invoke<PluginListResponse>("list_plugins");
        p0.setPluginsData(pl);
      } catch {
        // ignore
      }
      try {
        const sk = await invoke<SkillInfo[]>("list_skills");
        p0.setSkills(sk);
      } catch {
        // ignore
      }
      try {
        const sh = await invoke<ShareStatePayload>("get_share_state");
        p0.shareStateRef.current = sh;
        p0.setShareState(sh);
      } catch {
        // ignore
      }
      try {
        const bs = await invoke<BrowserStatePayload>("get_browser_state");
        p0.setBrowserState(bs.active ? bs : null);
      } catch {
        // ignore
      }
      let loadedLang: UiLang = "en";
      try {
        const cfg = await invoke<AppSettings>("get_settings");
        p0.setSettings(cfg);
        loadedLang = cfg.ui_language === "zh" ? "zh" : "en";
      } catch {
        // ignore
      }
      try {
        const convId = await invoke<number>("ensure_fresh_startup_conversation");
        await paramsRef.current.loadConversationMessages(convId, loadedLang);
        await paramsRef.current.refreshConversations();
      } catch {
        // ignore
      }
      paramsRef.current.applyVisualTargets("idle");
    })();

    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      listen<PeekPayload>("buddy://peek", (event) => {
        const ctx = paramsRef.current;
        if (
          ctx.stateRef.current === "expanded" ||
          ctx.stateRef.current === "capture" ||
          ctx.stateRef.current === "stealth"
        ) {
          return;
        }
        if (event.payload.hovering) {
          ctx.peekTargetRef.current = { dx: event.payload.dx, dy: event.payload.dy };
          if (ctx.stateRef.current !== "peek") {
            ctx.transitionTo("peek");
          }
        } else if (ctx.stateRef.current === "peek") {
          ctx.transitionTo(ctx.restStateOrMusic());
        }
      })
    );

    unlisteners.push(
      listen<StealthPayload>("buddy://stealth", (event) => {
        const ctx = paramsRef.current;
        if (event.payload.active) {
          ctx.transitionTo("stealth");
        } else if (ctx.stateRef.current === "stealth") {
          ctx.springRef.current.capY = 0;
          ctx.springRef.current.vCapY = 0;
          ctx.transitionTo("peek");
          window.setTimeout(() => {
            const c = paramsRef.current;
            if (c.stateRef.current === "peek") {
              c.transitionTo(c.restStateOrMusic());
            }
          }, 1300);
        }
      })
    );

    unlisteners.push(
      listen<SleepPayload>("buddy://idle-sleep", (event) => {
        const ctx = paramsRef.current;
        if (ctx.stateRef.current === "expanded" || ctx.stateRef.current === "stealth") return;
        if (event.payload.sleeping && ctx.musicRef.current.playing) return;
        ctx.transitionTo(event.payload.sleeping ? "sleep" : ctx.restStateOrMusic());
      })
    );

    unlisteners.push(
      listen<MusicPayload>("buddy://music", (event) => {
        const ctx = paramsRef.current;
        const p = event.payload;
        const wasPlaying = ctx.musicRef.current.playing;
        ctx.musicRef.current.playing = p.playing;
        ctx.musicRef.current.style = p.style;
        ctx.musicRef.current.bass = p.bass;
        ctx.musicRef.current.mid = p.mid;
        ctx.musicRef.current.high = p.high;
        if (p.beat) ctx.musicRef.current.beatImpulse = 1.0;

        if (wasPlaying !== p.playing) {
          void invoke("set_music_playing_flag", { playing: p.playing });
          if (ctx.stateRef.current === "peek" || ctx.stateRef.current === "expanded") {
            ctx.applyVisualTargets(ctx.stateRef.current);
          }
        }

        if (p.playing) {
          if (ctx.stateRef.current === "idle" || ctx.stateRef.current === "sleep") {
            ctx.transitionTo("music");
          }
        } else if (ctx.stateRef.current === "music") {
          ctx.transitionTo("idle");
        }
      })
    );

    unlisteners.push(
      listen<MemoryItem>("buddy://memory-updated", (event) => {
        const ctx = paramsRef.current;
        ctx.setMemoryRefresh((n) => n + 1);
        ctx.setRecentMemory(event.payload);
        if (ctx.recentTimerRef.current) window.clearTimeout(ctx.recentTimerRef.current);
        ctx.recentTimerRef.current = window.setTimeout(() => {
          paramsRef.current.setRecentMemory(null);
        }, 30_000);
      })
    );

    unlisteners.push(
      listen<AttachmentPayload[]>("buddy://files-dropped", (event) => {
        const ctx = paramsRef.current;
        ctx.setAttachments((prev) => [...prev, ...event.payload].slice(0, 6));
        const paths = event.payload
          .map((a) => a.file_path)
          .filter((p): p is string => Boolean(p));
        if (paths.length > 0) ctx.setDroppedPathsHint(paths);
        ctx.activeTabRef.current = "chat";
        ctx.setActiveTab("chat");
        if (ctx.stateRef.current !== "expanded") {
          ctx.transitionTo("expanded");
        }
      })
    );

    unlisteners.push(
      listen<AgentStep>("buddy://agent-step", (event) => {
        const ctx = paramsRef.current;
        const step = event.payload;
        if (ctx.activeStreamIdRef.current !== step.stream_id) return;
        ctx.setMessages((prev) =>
          prev.map((m) => {
            if (m.id.toString() !== step.stream_id) return m;
            const existing = m.steps ?? [];
            const idx = existing.findIndex((s) => s.step_id === step.step_id);
            const nextSteps =
              idx >= 0
                ? existing.map((s, i) => (i === idx ? step : s))
                : [...existing, step];
            return { ...m, steps: nextSteps };
          })
        );
        if (
          step.status === "waiting_approval" &&
          ctx.stateRef.current !== "expanded" &&
          ctx.stateRef.current !== "stealth"
        ) {
          const chatStr = getStrings(ctx.langRef.current).chat;
          ctx.presentBubble({
            kind: "reply_notice",
            tone: "approval",
            durationMs: 5_000,
            text:
              chatStr.approvalWaitingPrefix +
              formatBubblePreview(step.summary || step.tool_name, step.tool_name, 74),
          });
        }
      })
    );

    unlisteners.push(
      listen<{ stream_id: string; delta: string }>("buddy://ai-thought", (event) => {
        const ctx = paramsRef.current;
        const { stream_id, delta } = event.payload;
        if (ctx.activeStreamIdRef.current !== stream_id) return;
        ctx.setMessages((prev) =>
          prev.map((m) =>
            m.id.toString() === stream_id
              ? { ...m, thought: (m.thought ?? "") + delta }
              : m
          )
        );
      })
    );

    unlisteners.push(
      listen<{ stream_id: string; delta: string }>("buddy://ai-chunk", (event) => {
        const ctx = paramsRef.current;
        const { stream_id, delta } = event.payload;
        if (ctx.activeStreamIdRef.current !== stream_id) return;
        const tw = ctx.typewriterRef.current;
        if (tw.streamId === stream_id) {
          tw.chars.push(...Array.from(delta));
        }
      })
    );

    unlisteners.push(
      listen<{ stream_id: string; full_text: string }>("buddy://ai-done", (event) => {
        const ctx = paramsRef.current;
        if (ctx.activeStreamIdRef.current !== event.payload.stream_id) return;
        ctx.typewriterRef.current.streamDone = true;
        void ctx.refreshConversations();
        if (ctx.stateRef.current !== "expanded" && ctx.stateRef.current !== "stealth") {
          const chatStr = getStrings(ctx.langRef.current).chat;
          ctx.presentBubble({
            kind: "reply_notice",
            tone: "reply",
            durationMs: 5_000,
            text: formatBubblePreview(event.payload.full_text, chatStr.replyDoneFallback),
          });
        }
      })
    );

    unlisteners.push(
      listen<{ stream_id: string; error: string }>("buddy://ai-error", (event) => {
        const ctx = paramsRef.current;
        const { stream_id, error } = event.payload;
        if (ctx.activeStreamIdRef.current !== stream_id) return;
        ctx.activeStreamIdRef.current = null;
        ctx.typewriterRef.current.streamId = null;
        ctx.thinkingRef.current = false;
        ctx.setIsThinking(false);
        ctx.setActiveStreamId(null);
        if (ctx.stateRef.current === "expanded") {
          ctx.applyVisualTargets("expanded", false);
        } else if (ctx.stateRef.current !== "stealth") {
          const chatStr = getStrings(ctx.langRef.current).chat;
          ctx.presentBubble({
            kind: "reply_notice",
            tone: "error",
            durationMs: 5_000,
            text:
              chatStr.replyErrorPrefix +
              formatBubblePreview(error, chatStr.replyDoneFallback, 74),
          });
        }
        ctx.setMessages((prev) =>
          prev.map((m) =>
            m.id.toString() === stream_id ? { ...m, text: error, isError: true } : m
          )
        );
      })
    );

    unlisteners.push(
      listen("buddy://capture", () => paramsRef.current.triggerCapture()),
      listen("buddy://simulate-capture", () => paramsRef.current.triggerCapture()),
      listen("buddy://memory-cleaned", () => paramsRef.current.setMemoryRefresh((n) => n + 1)),
      listen("buddy://expand", () => paramsRef.current.transitionTo("expanded")),
      listen("buddy://collapse", () => {
        const ctx = paramsRef.current;
        if (ctx.stateRef.current === "expanded") ctx.transitionTo(ctx.restStateOrMusic());
      }),
      listen<boolean>("buddy://toggle-panel", (e) => {
        const ctx = paramsRef.current;
        if (e.payload) {
          ctx.activeTabRef.current = "chat";
          ctx.setActiveTab("chat");
          ctx.transitionTo("expanded");
          window.setTimeout(() => {
            window.dispatchEvent(new Event("buddy-focus-chat-input"));
          }, 45);
        } else if (ctx.stateRef.current === "expanded") {
          ctx.transitionTo(ctx.restStateOrMusic());
        }
      }),
      listen<boolean>("buddy://auto-hide-changed", (e) => {
        paramsRef.current.setSettings((prev) => ({ ...prev, auto_hide_stealth: e.payload }));
      }),
      listen<boolean>("buddy://music-reactive-changed", (e) => {
        paramsRef.current.setSettings((prev) => ({ ...prev, music_reactive: e.payload }));
      }),
      listen<DockStatePayload>("buddy://dock-changed", (e) => {
        const ctx = paramsRef.current;
        const edge = (e.payload.dock_edge || "top") as DockEdge;
        ctx.dockEdgeRef.current = edge;
        ctx.setDockEdge(edge);
        ctx.snapPreviewRef.current = null;
        ctx.setSnapPreview(null);
        ctx.applyVisualTargets(ctx.stateRef.current);
      }),
      listen<UpdateInfo>("buddy://update-available", (e) => {
        paramsRef.current.setAvailableUpdate(e.payload);
      }),
      listen<{ downloaded: number; total?: number | null }>("buddy://update-progress", (e) => {
        const total = e.payload.total ?? 0;
        paramsRef.current.setUpdateProgress(
          total > 0 ? Math.min(100, Math.round((e.payload.downloaded / total) * 100)) : 0
        );
      }),
      listen<PluginProgressPayload>("buddy://plugin-progress", (e) => {
        paramsRef.current.setPluginProgress((prev) => ({
          ...prev,
          [e.payload.plugin_id]: e.payload,
        }));
      }),
      listen("buddy://skills-updated", () => {
        void invoke<SkillInfo[]>("list_skills").then((sk) => paramsRef.current.setSkills(sk)).catch(() => {});
      }),
      listen<ColorPickedPayload>("buddy://color-picked", (e) => {
        const ctx = paramsRef.current;
        ctx.setLastPickedColor(e.payload);
        ctx.activeTabRef.current = "plugins";
        ctx.setActiveTab("plugins");
        ctx.setFocusedPluginId("color_picker");
        if (ctx.stateRef.current !== "expanded") {
          ctx.transitionTo("expanded");
        }
      }),
      listen<TimerTickPayload>("buddy://timer-tick", (e) => {
        paramsRef.current.setTimerState(e.payload.active ? e.payload : null);
      }),
      listen<string>("buddy://timer-done", () => {
        const ctx = paramsRef.current;
        ctx.setTimerState(null);
        ctx.renderEyeSvg("happy", 12);
        ctx.springRef.current.vCapScale = 2.2;
        window.setTimeout(() => {
          const c = paramsRef.current;
          if (c.stateRef.current !== "expanded") {
            c.applyVisualTargets(c.stateRef.current);
          }
        }, 1600);
      }),
      listen<ShareStatePayload>("buddy://share-state", (e) => {
        const ctx = paramsRef.current;
        const wasActive = Boolean(ctx.shareStateRef.current?.active);
        const nextShare = e.payload;
        ctx.shareStateRef.current = nextShare;
        ctx.setShareState(nextShare);
        if (!nextShare.active) {
          if (ctx.floatingBubbleRef.current?.kind === "share_whisper") {
            ctx.dismissBubbleAnimated();
          }
        } else if (ctx.stateRef.current === "sleep") {
          ctx.transitionTo("idle");
        }
        if (wasActive !== nextShare.active && ctx.stateRef.current !== "expanded") {
          ctx.applyVisualTargets(ctx.stateRef.current);
        }
      }),
      listen<ShareWhisperPayload>("buddy://share-whisper", (e) => {
        const ctx = paramsRef.current;
        const w = e.payload;
        if (ctx.stateRef.current === "expanded") {
          const cid = ctx.activeConversationIdRef.current;
          if (cid !== null) {
            void invoke("accept_share_whisper", { conversationId: cid, text: w.text }).catch(
              () => {}
            );
          }
          ctx.setMessages((prev) => [...prev, { id: Date.now(), role: "assistant", text: w.text }]);
        } else {
          ctx.presentBubble({
            kind: "share_whisper",
            whisper: w,
            durationMs: 14_000,
          });
        }
      }),
      listen<BrowserStatePayload>("buddy://browser-state", (e) => {
        const ctx = paramsRef.current;
        const st = e.payload;
        ctx.setBrowserState(st.active ? st : null);
        if (!st.active) {
          ctx.setBrowserPreview(null);
        }
      }),
      listen<BrowserPreviewPayload>("buddy://browser-preview", (e) => {
        paramsRef.current.setBrowserPreview(e.payload);
      }),
      listen<{ conversation_id?: number; source?: string; deleted?: boolean }>(
        "buddy://conversations-changed",
        (e) => {
          const ctx = paramsRef.current;
          void ctx.refreshConversations();
          const cid = e.payload?.conversation_id;
          if (
            cid &&
            cid === ctx.activeConversationIdRef.current &&
            !e.payload?.deleted &&
            !ctx.activeStreamIdRef.current
          ) {
            void ctx.loadConversationMessages(cid, ctx.langRef.current);
          }
        }
      )
    );

    const onKeyDown = (e: KeyboardEvent) => {
      const ctx = paramsRef.current;
      if (e.key === "Escape" && ctx.stateRef.current === "expanded") {
        ctx.transitionTo(ctx.restStateOrMusic());
      }
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      window.removeEventListener("keydown", onKeyDown);
      unlisteners.forEach((p) => p.then((un) => un()));
    };
  }, []);
}
