import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clock, Download, Loader2, MessageSquare, Pause, Pin, PinOff, Play, Puzzle, Settings, Square, X } from "lucide-react";
import ChatTab, { memoryToAttachment } from "./components/ChatTab";
const MemoryTab = lazy(() => import("./components/MemoryTab"));
const PluginsTab = lazy(() => import("./components/PluginsTab"));
const SettingsTab = lazy(() => import("./components/SettingsTab"));
import { getStrings, makeWelcomeGreeting } from "./i18n";
import {
  clampThinkingLevel,
  getModelThinkingCapability,
  normalizeThinkingLevel,
  resolveActiveChatModel,
} from "./thinking";
import type {
  AgentPermissionMode,
  AgentStep,
  AppSettings,
  ApprovalDecision,
  AttachmentPayload,
  BuddyState,
  ChatMessage,
  ColorPickedPayload,
  ConversationSummary,
  DockEdge,
  DockStatePayload,
  LayoutDims,
  MemoryItem,
  MusicPayload,
  MusicStyle,
  OcrResult,
  PluginListResponse,
  PluginProgressPayload,
  PluginToolOutput,
  ShareStatePayload,
  ShareWhisperPayload,
  SkillInfo,
  TabId,
  ThinkingLevel,
  TimerTickPayload,
  UiLang,
  UpdateInfo,
} from "./types";

type FloatingBubble =
  | { kind: "share_whisper"; whisper: ShareWhisperPayload; durationMs: number }
  | {
      kind: "reply_notice";
      text: string;
      tone: "reply" | "error" | "approval";
      durationMs: number;
    };

function formatBubblePreview(raw: string, fallback: string, maxChars = 88): string {
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

type IdleBehavior =
  | "long_doze"
  | "settle_drowsy"
  | "startled_wake"
  | "daydream_dots"
  | "eavesdrop"
  | "yawn"
  | "look_around"
  | "wink_play";

type EyePreset = "open" | "doze" | "sleep" | "happy" | "lofi" | "pop" | "edm" | "thinking";

interface EyePose {
  pillW: number;
  pillH: number;
  pillOp: number;
  glintOp: number;
  arcBend: number; // + = closed sleep U-curve, - = happy/pop ^-curve
  arcOp: number;
  glowOp: number;
  isHappyGreen: boolean;
}

const EYE_POSES: Record<EyePreset, EyePose> = {
  open: {
    pillW: 26, pillH: 34, pillOp: 1, glintOp: 0.45,
    arcBend: 0, arcOp: 0, glowOp: 0, isHappyGreen: false,
  },
  thinking: {
    pillW: 26, pillH: 34, pillOp: 1, glintOp: 0.48,
    arcBend: 0, arcOp: 0, glowOp: 0.16, isHappyGreen: false,
  },
  lofi: {
    pillW: 29, pillH: 15, pillOp: 0.96, glintOp: 0.34,
    arcBend: 0, arcOp: 0, glowOp: 0.08, isHappyGreen: false,
  },
  doze: {
    pillW: 30, pillH: 9.5, pillOp: 0.92, glintOp: 0.12,
    arcBend: 0, arcOp: 0, glowOp: 0, isHappyGreen: false,
  },
  edm: {
    pillW: 26, pillH: 31, pillOp: 1, glintOp: 0.55,
    arcBend: 0, arcOp: 0, glowOp: 0.34, isHappyGreen: false,
  },
  sleep: {
    pillW: 30, pillH: 7, pillOp: 0, glintOp: 0,
    arcBend: 11, arcOp: 0.65, glowOp: 0, isHappyGreen: false,
  },
  pop: {
    pillW: 28, pillH: 8, pillOp: 0, glintOp: 0,
    arcBend: -15, arcOp: 0.96, glowOp: 0.1, isHappyGreen: false,
  },
  happy: {
    pillW: 28, pillH: 8, pillOp: 0, glintOp: 0,
    arcBend: -15, arcOp: 1.0, glowOp: 0.14, isHappyGreen: false,
  },
};

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

interface StoredChatRow {
  id: number;
  role: "user" | "assistant";
  content: string;
  attachments_json: string | null;
  agent_steps_json: string | null;
  thought_text: string | null;
}

function stepSpring(
  val: number,
  vel: number,
  target: number,
  stiffness: number,
  damping: number,
  dt: number
): [number, number] {
  const force = (target - val) * stiffness;
  const accel = force - vel * damping;
  const nextVel = vel + accel * dt;
  const nextVal = val + nextVel * dt;
  return [nextVal, nextVel];
}

function makeWelcomeMessage(lang: UiLang): ChatMessage {
  return {
    id: 1,
    role: "assistant",
    text: makeWelcomeGreeting(lang),
  };
}

export default function App() {
  const [buddyState, setBuddyState] = useState<BuddyState>("idle");
  const [activeTab, setActiveTab] = useState<TabId>("chat");
  const [settings, setSettings] = useState<AppSettings>({
    api_provider: "openai",
    base_url: "https://api.xiaomimimo.com/v1",
    model_id: "mimo-v2-flash",
    persona_id: "warm_companion",
    auto_hide_stealth: false,
    music_reactive: true,
    launch_at_startup: false,
    has_api_key: false,
    agent_mode: true,
    thinking_level: "medium",
    agent_workspace: "",
    ui_language: "en",
    auto_collapse_on_blur: false,
    agent_permission_mode: "default",
  });
  const [layoutDims, setLayoutDims] = useState<LayoutDims>({
    v_w: 420,
    v_h: 460,
    h_w: 456,
    h_h: 460,
    canvas_w: 472,
    canvas_h: 476,
    pad: 8,
  });
  const [dockEdge, setDockEdge] = useState<DockEdge>("top");
  const [snapPreview, setSnapPreview] = useState<DockEdge | null>(null);
  const [appVersion, setAppVersion] = useState<string>("0.1.0");
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [pluginsData, setPluginsData] = useState<PluginListResponse>({
    plugins: [],
    mirror_prefix: "",
    total_size_bytes: 0,
  });
  const [pluginProgress, setPluginProgress] = useState<Record<string, PluginProgressPayload>>({});
  const [lastPickedColor, setLastPickedColor] = useState<ColorPickedPayload | null>(null);
  const [timerState, setTimerState] = useState<TimerTickPayload | null>(null);
  const [shareState, setShareState] = useState<ShareStatePayload | null>(null);
  const [floatingBubble, setFloatingBubble] = useState<FloatingBubble | null>(null);
  const [bubbleVisible, setBubbleVisible] = useState(false);
  const [whisperHovered, setWhisperHovered] = useState(false);
  const [droppedPathsHint, setDroppedPathsHint] = useState<string[]>([]);
  const [focusedPluginId, setFocusedPluginId] = useState<string | null>(null);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [refreshHz, setRefreshHz] = useState<number>(60);
  const [memoryRefresh, setMemoryRefresh] = useState(0);
  const [attachments, setAttachments] = useState<AttachmentPayload[]>([]);
  const [recentMemory, setRecentMemory] = useState<MemoryItem | null>(null);

  const [messages, setMessages] = useState<ChatMessage[]>(() => [makeWelcomeMessage("en")]);
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [activeConversationId, setActiveConversationId] = useState<number | null>(null);
  const [chatInput, setChatInput] = useState("");
  const [isThinking, setIsThinking] = useState(false);
  const [activeStreamId, setActiveStreamId] = useState<string | null>(null);

  const stateRef = useRef<BuddyState>("idle");
  const shareStateRef = useRef<ShareStatePayload | null>(null);
  const whisperRef = useRef<HTMLDivElement | null>(null);
  const whisperTimerRef = useRef<number | null>(null);
  const bubbleExitTimerRef = useRef<number | null>(null);
  const floatingBubbleRef = useRef<FloatingBubble | null>(null);
  const activeConversationIdRef = useRef<number | null>(null);
  const focusedRef = useRef(false);
  const thinkingRef = useRef(false);
  const thinkingLevelRef = useRef<ThinkingLevel>("medium");
  const langRef = useRef<UiLang>("en");
  const activeStreamIdRef = useRef<string | null>(null);
  const typewriterRef = useRef<{
    streamId: string | null;
    chars: string[];
    shownLen: number;
    streamDone: boolean;
    acc: number;
    lastTick: number;
  }>({
    streamId: null,
    chars: [],
    shownLen: 0,
    streamDone: false,
    acc: 0,
    lastTick: 0,
  });
  const layoutDimsRef = useRef<LayoutDims>({
    v_w: 420,
    v_h: 460,
    h_w: 456,
    h_h: 460,
    canvas_w: 472,
    canvas_h: 476,
    pad: 8,
  });
  const dockEdgeRef = useRef<DockEdge>("top");
  const snapPreviewRef = useRef<DockEdge | null>(null);
  const isDraggingRef = useRef(false);
  const dragStepInFlightRef = useRef(false);
  const dragMetaRef = useRef<{
    active: boolean;
    pointerId: number;
    startX: number;
    startY: number;
    moved: boolean;
  } | null>(null);
  const activeTabRef = useRef<TabId>("chat");
  const peekTargetRef = useRef({ dx: 0, dy: 0 });
  const captureTimerRef = useRef<number | null>(null);
  const recentTimerRef = useRef<number | null>(null);

  const musicRef = useRef<{
    playing: boolean;
    style: MusicStyle;
    bass: number;
    mid: number;
    high: number;
    beatImpulse: number;
    renderedStyle: MusicStyle;
    eqHeights: number[];
    phase: number;
  }>({
    playing: false,
    style: "lofi",
    bass: 0,
    mid: 0,
    high: 0,
    beatImpulse: 0,
    renderedStyle: "lofi",
    eqHeights: [4, 6, 8, 5],
    phase: 0,
  });

  // GPU-composited DOM & SVG refs
  const morphLayerRef = useRef<HTMLDivElement | null>(null);
  const pillShadowRef = useRef<SVGRectElement | null>(null);
  const pillRectRef = useRef<SVGRectElement | null>(null);
  const clipBoxRef = useRef<HTMLDivElement | null>(null);
  const cardFrameRef = useRef<HTMLDivElement | null>(null);
  const eyesStageRef = useRef<HTMLDivElement | null>(null);
  const facePivotRef = useRef<HTMLDivElement | null>(null);
  const eyesPairRef = useRef<HTMLDivElement | null>(null);
  const leftEyeShapeRef = useRef<SVGGElement | null>(null);
  const rightEyeShapeRef = useRef<SVGGElement | null>(null);
  const leftEyeGlowRef = useRef<SVGEllipseElement | null>(null);
  const rightEyeGlowRef = useRef<SVGEllipseElement | null>(null);
  const leftEyePillRef = useRef<SVGRectElement | null>(null);
  const rightEyePillRef = useRef<SVGRectElement | null>(null);
  const leftEyeGlintRef = useRef<SVGEllipseElement | null>(null);
  const rightEyeGlintRef = useRef<SVGEllipseElement | null>(null);
  const leftEyeArcRef = useRef<SVGPathElement | null>(null);
  const rightEyeArcRef = useRef<SVGPathElement | null>(null);
  const mouthSvgRef = useRef<SVGSVGElement | null>(null);
  const mouthShapeRef = useRef<SVGGElement | null>(null);
  const particlesRef = useRef<HTMLDivElement | null>(null);
  const particleElsRef = useRef<(HTMLSpanElement | null)[]>([]);
  const sleepZRef = useRef<HTMLDivElement | null>(null);
  const idleDotsRef = useRef<HTMLDivElement | null>(null);
  const musicNoteRef = useRef<HTMLDivElement | null>(null);
  const musicEqRef = useRef<HTMLDivElement | null>(null);
  const eqBarRefs = useRef<(HTMLSpanElement | null)[]>([]);
  const captureToastRef = useRef<HTMLDivElement | null>(null);

  const springRef = useRef({
    w: 152, vw: 0, tw: 152,
    h: 42, vh: 0, th: 42,
    r: 21, vr: 0, tr: 21,
    ex: 0, vex: 0, tex: 0,
    ey: 0, vey: 0, tey: 0,
    eRot: 0, veRot: 0, teRot: 0,
    esx: 1, vesx: 0, tesx: 1,
    esy: 1, vesy: 0, tesy: 1,
    capScale: 1, vCapScale: 0, tCapScale: 1,
    capY: 0, vCapY: 0, tCapY: 0,
    capRot: 0, vCapRot: 0, tCapRot: 0,
    eyeSize: 26, vEyeSize: 0, tEyeSize: 26,
    stageH: 42, vStageH: 0, tStageH: 42,
    stageW: 42, vStageW: 0, tStageW: 42,
    headRot: 0, vHeadRot: 0, tHeadRot: 0,
  });

  const animFlagsRef = useRef({
    captureElapsed: 0,
    capturePhase: "inhale" as "inhale" | "swallow",
    idleBehavior: "long_doze" as IdleBehavior,
    idleElapsed: 0,
    idleDuration: 8.5,
    idleDir: 1,
    idleEyeMode: "doze" as "doze" | "open" | "sleep",
    blinkTimer: 0,
    nextBlinkAt: 2.8,
    blinkProgress: 0,
    winkTimer: 0,
    winkProgress: 0,
    wasDrowsy: true,
    wakeKind: "none" as "none" | "from_sleep" | "from_doze",
    wakeElapsed: 0,
    wakeDuration: 0.65,
    eyeSpeed: 10,
  });

  const eyeMorphRef = useRef<{ cur: EyePose; target: EyePose }>({
    cur: { ...EYE_POSES.doze },
    target: { ...EYE_POSES.doze },
  });

  const applyEyePoseAttributes = (pose: EyePose) => {
    const px = 22 - pose.pillW / 2;
    const py = 22 - pose.pillH / 2;
    const prx = Math.min(pose.pillW / 2, pose.pillH / 2);
    const pillFill = pose.isHappyGreen ? "#6ee7b7" : "#ffffff";
    const arcStroke = pose.isHappyGreen ? "#6ee7b7" : "#ffffff";
    const glintCy = Math.max(10, py + pose.pillH * 0.28);
    const glintRy = Math.min(4.5, Math.max(0.8, pose.pillH * 0.14));
    const glintFade = Math.min(1, Math.max(0, (pose.pillH - 7) / 14));
    const effectiveGlintOp = pose.glintOp * glintFade * Math.min(1, pose.pillOp);
    const glowRx = Math.max(5, pose.pillW * 0.5 + 1.5);
    const glowRy =
      pose.pillOp < 0.2 ? 5.5 : Math.max(4.5, pose.pillH * 0.5 + 1.5);
    const effectiveGlowOp = pose.pillOp < 0.2 ? 0 : pose.glowOp;
    const dStr = `M 8 22 Q 22 ${(22 + pose.arcBend).toFixed(2)} 36 22`;

    const applySide = (
      glow: SVGEllipseElement | null,
      pill: SVGRectElement | null,
      glint: SVGEllipseElement | null,
      arc: SVGPathElement | null
    ) => {
      if (glow) {
        glow.setAttribute("rx", glowRx.toFixed(2));
        glow.setAttribute("ry", glowRy.toFixed(2));
        glow.setAttribute("opacity", effectiveGlowOp.toFixed(2));
        glow.setAttribute("fill", "#ffffff");
      }
      if (pill) {
        pill.setAttribute("x", px.toFixed(2));
        pill.setAttribute("y", py.toFixed(2));
        pill.setAttribute("width", pose.pillW.toFixed(2));
        pill.setAttribute("height", Math.max(2, pose.pillH).toFixed(2));
        pill.setAttribute("rx", Math.max(1, prx).toFixed(2));
        pill.setAttribute("fill", pillFill);
        pill.setAttribute("opacity", pose.pillOp.toFixed(2));
      }
      if (glint) {
        glint.setAttribute("cy", glintCy.toFixed(2));
        glint.setAttribute("ry", glintRy.toFixed(2));
        glint.setAttribute("opacity", effectiveGlintOp.toFixed(2));
      }
      if (arc) {
        arc.setAttribute("d", dStr);
        arc.setAttribute("stroke", arcStroke);
        arc.setAttribute("opacity", pose.arcOp.toFixed(2));
      }
    };

    applySide(
      leftEyeGlowRef.current,
      leftEyePillRef.current,
      leftEyeGlintRef.current,
      leftEyeArcRef.current
    );
    applySide(
      rightEyeGlowRef.current,
      rightEyePillRef.current,
      rightEyeGlintRef.current,
      rightEyeArcRef.current
    );
  };

  const renderEyeSvg = (type: EyePreset, speed = 10) => {
    eyeMorphRef.current.target = { ...EYE_POSES[type] };
    animFlagsRef.current.eyeSpeed = speed;
  };

  const isSideDock = (edge: DockEdge) => edge === "left" || edge === "right";
  const headTargetDeg = (edge: DockEdge) =>
    edge === "left" ? 90 : edge === "right" ? -90 : 0;

  const setCapsuleSizeTarget = (
    horzW: number,
    horzH: number,
    r: number,
    stageBreadth = horzH,
    updateHitbox = true
  ) => {
    const s = springRef.current;
    const side = isSideDock(dockEdgeRef.current);
    s.tw = side ? horzH : horzW;
    s.th = side ? horzW : horzH;
    s.tr = r;
    s.tStageH = side ? layoutDimsRef.current.h_h : stageBreadth;
    s.tStageW = side ? stageBreadth : layoutDimsRef.current.v_w;
    s.tHeadRot = headTargetDeg(dockEdgeRef.current);
    if (updateHitbox) {
      void invoke("update_pill_bounds", {
        width: Math.round(s.tw),
        height: Math.round(s.th),
      });
    }
  };

  const syncMusicOverlay = (state: BuddyState) => {
    const showMusic =
      musicRef.current.playing &&
      (state === "music" || state === "peek" || state === "expanded");
    if (!musicNoteRef.current || !musicEqRef.current) return;

    if (!showMusic) {
      musicNoteRef.current.classList.add("hidden");
      musicEqRef.current.classList.add("hidden");
      return;
    }

    musicNoteRef.current.classList.remove("hidden");
    musicEqRef.current.classList.remove("hidden");
    const side = isSideDock(dockEdgeRef.current);

    if (state === "expanded") {
      musicNoteRef.current.style.left = side ? "calc(50% - 92px)" : "calc(50% - 110px)";
      musicNoteRef.current.style.top = "50%";
      musicNoteRef.current.style.right = "auto";
      musicNoteRef.current.style.fontSize = side ? "19px" : "22px";
      musicEqRef.current.style.left = side ? "calc(50% + 68px)" : "calc(50% + 84px)";
      musicEqRef.current.style.right = "auto";
      musicEqRef.current.style.top = "50%";
      musicEqRef.current.style.gap = side ? "3px" : "4px";
      musicEqRef.current.style.height = side ? "26px" : "32px";
    } else {
      musicNoteRef.current.style.left = "calc(50% - 76px)";
      musicNoteRef.current.style.top = "50%";
      musicNoteRef.current.style.right = "auto";
      musicNoteRef.current.style.fontSize = "15px";
      musicEqRef.current.style.left = "calc(50% + 52px)";
      musicEqRef.current.style.right = "auto";
      musicEqRef.current.style.top = "50%";
      musicEqRef.current.style.gap = "3px";
      musicEqRef.current.style.height = "22px";
    }
  };

  const restStateOrMusic = (): BuddyState =>
    musicRef.current.playing ? "music" : "idle";

  const pickNextIdleBehavior = (prev: IdleBehavior) => {
    const f = animFlagsRef.current;
    const pool: { kind: IdleBehavior; weight: number; min: number; max: number }[] = [
      { kind: "long_doze", weight: 36, min: 7.5, max: 13.5 },
      { kind: "eavesdrop", weight: 16, min: 2.8, max: 4.4 },
      { kind: "daydream_dots", weight: 14, min: 3.0, max: 4.5 },
      { kind: "look_around", weight: 14, min: 2.6, max: 4.0 },
      { kind: "yawn", weight: 10, min: 2.2, max: 2.8 },
      { kind: "wink_play", weight: 6, min: 1.6, max: 2.2 },
      {
        kind: "startled_wake",
        weight: prev === "long_doze" ? 20 : 4,
        min: 1.2,
        max: 1.8,
      },
    ];
    const candidates = pool.filter((p) => p.kind !== prev);
    const total = candidates.reduce((sum, c) => sum + c.weight, 0);
    let r = Math.random() * total;
    let chosen = candidates[0];
    for (const c of candidates) {
      r -= c.weight;
      if (r <= 0) {
        chosen = c;
        break;
      }
    }
    f.idleBehavior = chosen.kind;
    f.idleElapsed = 0;
    f.idleDuration = chosen.min + Math.random() * (chosen.max - chosen.min);
    f.idleDir = Math.random() < 0.5 ? -1 : 1;

    idleDotsRef.current?.classList.add("hidden");
    mouthSvgRef.current?.classList.add("hidden");
    const s = springRef.current;
    s.tCapRot = 0;
    s.tesx = 1;
    s.tesy = 1;

    if (chosen.kind === "long_doze") {
      f.idleEyeMode = "doze";
      renderEyeSvg("doze", 4.5);
      setCapsuleSizeTarget(152, 42, 21, 42);
    } else if (chosen.kind === "yawn") {
      f.idleEyeMode = "doze";
      renderEyeSvg("doze", 7.0);
      mouthSvgRef.current?.classList.remove("hidden");
      if (mouthShapeRef.current) mouthShapeRef.current.innerHTML = "";
      setCapsuleSizeTarget(152, 60, 21, 60);
    } else {
      f.idleEyeMode = "open";
      renderEyeSvg("open", chosen.kind === "startled_wake" ? 16.0 : 8.5);
      setCapsuleSizeTarget(152, 44, 21, 44);
      if (chosen.kind === "daydream_dots") {
        if (idleDotsRef.current) {
          idleDotsRef.current.style.top = "";
          idleDotsRef.current.style.transform = "";
          idleDotsRef.current.classList.remove("hidden");
        }
      }
      if (chosen.kind === "startled_wake") {
        s.vCapScale = 1.4;
      }
    }
  };

  const presentBubble = (next: FloatingBubble) => {
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
      bubbleExitTimerRef.current = null;
    }
    floatingBubbleRef.current = next;
    setBubbleVisible(false);
    setFloatingBubble(next);
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        setBubbleVisible(true);
      });
    });
  };

  const dismissBubbleAnimated = () => {
    if (whisperTimerRef.current) {
      window.clearTimeout(whisperTimerRef.current);
      whisperTimerRef.current = null;
    }
    setBubbleVisible(false);
    setWhisperHovered(false);
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
    }
    bubbleExitTimerRef.current = window.setTimeout(() => {
      bubbleExitTimerRef.current = null;
      floatingBubbleRef.current = null;
      setFloatingBubble(null);
    }, 220);
  };

  const applyVisualTargets = (next: BuddyState, thinkingOverride?: boolean) => {
    const s = springRef.current;
    const f = animFlagsRef.current;
    const thinking = thinkingOverride ?? thinkingRef.current;

    s.tCapY = 0;
    s.tCapScale = 1;
    s.tCapRot = 0;
    s.teRot = 0;
    s.tesx = 1;
    s.tesy = 1;
    s.tHeadRot = headTargetDeg(dockEdgeRef.current);

    sleepZRef.current?.classList.add("hidden");
    idleDotsRef.current?.classList.add("hidden");
    mouthSvgRef.current?.classList.add("hidden");
    particlesRef.current?.classList.add("hidden");
    captureToastRef.current?.classList.add("hidden");
    captureToastRef.current?.classList.remove("flex");
    syncMusicOverlay(next);

    if (next === "idle") {
      const sharing = Boolean(shareStateRef.current?.active);
      s.tEyeSize = 26;
      setCapsuleSizeTarget(sharing ? 224 : 152, 42, 21, 42);
      f.idleBehavior = sharing ? "look_around" : "settle_drowsy";
      f.idleElapsed = 0;
      f.idleDuration = sharing ? 4.0 : f.wasDrowsy ? 2.6 : 3.2;
      f.idleEyeMode = "open";
      renderEyeSvg("open", sharing ? 9.5 : f.wasDrowsy ? 7.5 : 8.0);
    } else if (next === "sleep") {
      s.tEyeSize = 24;
      setCapsuleSizeTarget(138, 38, 19, 38);
      f.wasDrowsy = true;
      renderEyeSvg("sleep", 4.0);
      sleepZRef.current?.classList.remove("hidden");
    } else if (next === "capture") {
      f.captureElapsed = 0;
      f.capturePhase = "inhale";
      f.wakeKind = "none";
      s.tEyeSize = 26;
      setCapsuleSizeTarget(176, 58, 25, 58, false);
      s.vCapScale = 1.25;
      renderEyeSvg("open", 16.0);
      mouthSvgRef.current?.classList.remove("hidden");
      particlesRef.current?.classList.remove("hidden");
      if (mouthShapeRef.current) {
        mouthShapeRef.current.innerHTML = `<ellipse cx="18" cy="12" rx="4.5" ry="4.5" fill="#ffffff" opacity="0.95" />`;
      }
      const side = isSideDock(dockEdgeRef.current);
      void invoke("update_pill_bounds", {
        width: side ? 64 : 230,
        height: side ? 230 : 64,
      });
    } else if (next === "peek") {
      const sharing = Boolean(shareStateRef.current?.active);
      if (musicRef.current.playing) {
        s.tEyeSize = 26;
        setCapsuleSizeTarget(sharing ? 236 : 204, 48, 24, 48);
        musicRef.current.renderedStyle = musicRef.current.style;
        if (f.wakeKind === "none") {
          renderEyeSvg("open", 10.0);
        }
      } else {
        s.tEyeSize = 30;
        setCapsuleSizeTarget(sharing ? 236 : 186, 50, 25, 50);
        if (f.wakeKind === "none") {
          renderEyeSvg("open", 10.0);
        }
      }
    } else if (next === "music") {
      s.tEyeSize = 26;
      setCapsuleSizeTarget(196, 44, 22, 44);
      musicRef.current.renderedStyle = musicRef.current.style;
      if (f.wakeKind === "none") {
        renderEyeSvg("open", 8.5);
      }
    } else if (next === "expanded") {
      const dims = layoutDimsRef.current;
      const side = isSideDock(dockEdgeRef.current);
      s.tw = side ? dims.h_w : dims.v_w;
      s.th = side ? dims.h_h : dims.v_h;
      s.tr = 26;
      s.tEyeSize = side ? 34 : 46;
      s.tStageH = side ? dims.h_h : 108;
      s.tStageW = side ? 64 : dims.v_w;
      if (thinking) {
        renderEyeSvg("thinking", 12.0);
      } else if (f.wakeKind === "none") {
        renderEyeSvg("open", 9.0);
      }
    } else if (next === "stealth") {
      setCapsuleSizeTarget(148, 42, 21, 42, false);
      s.tCapY = 0;
    }
  };

  const transitionTo = (next: BuddyState) => {
    const prev = stateRef.current;
    const f = animFlagsRef.current;

    // Trigger natural slow wake-up whenever waking from sleep/doze (peek, expand, or music start)
    if (
      (next === "peek" || next === "expanded" || next === "music") &&
      prev !== "expanded" &&
      prev !== "music"
    ) {
      const prevWasSleep = prev === "sleep";
      const prevWasDoze =
        prev === "idle" && (f.idleBehavior === "long_doze" || f.idleBehavior === "yawn");
      if (prevWasSleep) {
        f.wasDrowsy = true;
        f.wakeKind = "from_sleep";
        f.wakeElapsed = 0;
        f.wakeDuration = 0.85;
      } else if (prevWasDoze) {
        f.wasDrowsy = true;
        f.wakeKind = "from_doze";
        f.wakeElapsed = 0;
        f.wakeDuration = 0.62;
      } else if (prev !== "peek") {
        f.wasDrowsy = false;
        f.wakeKind = "none";
      }
    } else if (next === "capture") {
      f.wasDrowsy = false;
      f.wakeKind = "none";
    } else if (next === "idle" && prev === "expanded") {
      f.wakeKind = "none";
      f.wasDrowsy = true;
    } else if (next === "idle" && prev === "peek") {
      f.wakeKind = "none";
    } else if (next === "idle" && prev === "music") {
      f.wakeKind = "none";
      f.wasDrowsy = false;
    }

    stateRef.current = next;
    setBuddyState(next);

    if (next === "expanded" && prev !== "expanded") {
      switchTab("chat");
      void invoke<LayoutDims>("set_expanded", { expanded: true })
        .then((dims) => {
          layoutDimsRef.current = dims;
          setLayoutDims(dims);
          applyVisualTargets("expanded");
        })
        .catch(() => {});
    } else if (next !== "expanded" && prev === "expanded") {
      void invoke<LayoutDims>("set_expanded", { expanded: false })
        .then((dims) => {
          layoutDimsRef.current = dims;
          setLayoutDims(dims);
        })
        .catch(() => {});
    }

    applyVisualTargets(next);
  };

  const triggerCapture = () => {
    if (stateRef.current === "stealth" || stateRef.current === "expanded") return;
    if (captureTimerRef.current) window.clearTimeout(captureTimerRef.current);
    transitionTo("capture");
    captureTimerRef.current = window.setTimeout(() => {
      if (stateRef.current === "capture") {
        transitionTo(restStateOrMusic());
      }
    }, 1650);
  };

  const switchTab = (tab: TabId) => {
    activeTabRef.current = tab;
    setActiveTab(tab);
    try {
      localStorage.setItem("buddy_active_tab", tab);
    } catch {
      // ignore
    }
  };

  const loadConversationMessages = async (convId: number, langOverride?: UiLang) => {
    activeConversationIdRef.current = convId;
    setActiveConversationId(convId);
    const curLang: UiLang = langOverride ?? (settings.ui_language === "zh" ? "zh" : "en");
    try {
      const rows = await invoke<StoredChatRow[]>("list_chat_history", {
        conversationId: convId,
      });
      if (rows.length > 0) {
        setMessages(
          rows.map((r) => ({
            id: r.id,
            role: r.role,
            text: r.content,
            thought: r.thought_text ?? undefined,
            attachments: r.attachments_json
              ? (JSON.parse(r.attachments_json) as AttachmentPayload[])
              : undefined,
            steps: r.agent_steps_json
              ? (JSON.parse(r.agent_steps_json) as AgentStep[])
              : undefined,
          }))
        );
      } else {
        setMessages([makeWelcomeMessage(curLang)]);
      }
    } catch {
      setMessages([makeWelcomeMessage(curLang)]);
    }
  };

  const refreshConversations = async () => {
    try {
      const list = await invoke<ConversationSummary[]>("list_conversations");
      setConversations(list);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    void (async () => {
      try {
        const dims = await invoke<LayoutDims>("get_expanded_bounds");
        layoutDimsRef.current = dims;
        setLayoutDims(dims);
      } catch {
        // ignore
      }
      try {
        const dock = await invoke<DockStatePayload>("get_dock_state");
        const edge = (dock.dock_edge || "top") as DockEdge;
        dockEdgeRef.current = edge;
        setDockEdge(edge);
        const deg = headTargetDeg(edge);
        springRef.current.headRot = deg;
        springRef.current.tHeadRot = deg;
        const side = isSideDock(edge);
        springRef.current.w = side ? 42 : 152;
        springRef.current.h = side ? 152 : 42;
      } catch {
        // ignore
      }
      try {
        const ver = await invoke<string>("get_app_version");
        setAppVersion(ver);
      } catch {
        // ignore
      }
      try {
        const pl = await invoke<PluginListResponse>("list_plugins");
        setPluginsData(pl);
      } catch {
        // ignore
      }
      try {
        const sk = await invoke<SkillInfo[]>("list_skills");
        setSkills(sk);
      } catch {
        // ignore
      }
      try {
        const sh = await invoke<ShareStatePayload>("get_share_state");
        shareStateRef.current = sh;
        setShareState(sh);
      } catch {
        // ignore
      }
      let loadedLang: UiLang = "en";
      try {
        const cfg = await invoke<AppSettings>("get_settings");
        setSettings(cfg);
        loadedLang = cfg.ui_language === "zh" ? "zh" : "en";
      } catch {
        // ignore
      }
      try {
        const convId = await invoke<number>("ensure_fresh_startup_conversation");
        await loadConversationMessages(convId, loadedLang);
        await refreshConversations();
      } catch {
        // ignore
      }
      applyVisualTargets("idle");
    })();

    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      listen<PeekPayload>("buddy://peek", (event) => {
        if (
          stateRef.current === "expanded" ||
          stateRef.current === "capture" ||
          stateRef.current === "stealth"
        ) {
          return;
        }
        if (event.payload.hovering) {
          peekTargetRef.current = { dx: event.payload.dx, dy: event.payload.dy };
          if (stateRef.current !== "peek") {
            transitionTo("peek");
          }
        } else if (stateRef.current === "peek") {
          transitionTo(restStateOrMusic());
        }
      })
    );

    unlisteners.push(
      listen<StealthPayload>("buddy://stealth", (event) => {
        if (event.payload.active) {
          transitionTo("stealth");
        } else if (stateRef.current === "stealth") {
          springRef.current.capY = 0;
          springRef.current.vCapY = 0;
          transitionTo("peek");
          window.setTimeout(() => {
            if (stateRef.current === "peek") {
              transitionTo(restStateOrMusic());
            }
          }, 1300);
        }
      })
    );

    unlisteners.push(
      listen<SleepPayload>("buddy://idle-sleep", (event) => {
        if (stateRef.current === "expanded" || stateRef.current === "stealth") return;
        if (event.payload.sleeping && musicRef.current.playing) return;
        transitionTo(event.payload.sleeping ? "sleep" : restStateOrMusic());
      })
    );

    unlisteners.push(
      listen<MusicPayload>("buddy://music", (event) => {
        const p = event.payload;
        const wasPlaying = musicRef.current.playing;
        musicRef.current.playing = p.playing;
        musicRef.current.style = p.style;
        musicRef.current.bass = p.bass;
        musicRef.current.mid = p.mid;
        musicRef.current.high = p.high;
        if (p.beat) musicRef.current.beatImpulse = 1.0;

        if (wasPlaying !== p.playing) {
          void invoke("set_music_playing_flag", { playing: p.playing });
          if (stateRef.current === "peek" || stateRef.current === "expanded") {
            applyVisualTargets(stateRef.current);
          }
        }

        if (p.playing) {
          if (stateRef.current === "idle" || stateRef.current === "sleep") {
            transitionTo("music");
          }
        } else if (stateRef.current === "music") {
          transitionTo("idle");
        }
      })
    );

    unlisteners.push(
      listen<MemoryItem>("buddy://memory-updated", (event) => {
        setMemoryRefresh((n) => n + 1);
        setRecentMemory(event.payload);
        if (recentTimerRef.current) window.clearTimeout(recentTimerRef.current);
        recentTimerRef.current = window.setTimeout(() => {
          setRecentMemory(null);
        }, 30_000);
      })
    );

    unlisteners.push(
      listen<AttachmentPayload[]>("buddy://files-dropped", (event) => {
        setAttachments((prev) => [...prev, ...event.payload].slice(0, 6));
        const paths = event.payload
          .map((a) => a.file_path)
          .filter((p): p is string => Boolean(p));
        if (paths.length > 0) setDroppedPathsHint(paths);
        activeTabRef.current = "chat";
        setActiveTab("chat");
        if (stateRef.current !== "expanded") {
          transitionTo("expanded");
        }
      })
    );

    unlisteners.push(
      listen<AgentStep>("buddy://agent-step", (event) => {
        const step = event.payload;
        if (activeStreamIdRef.current !== step.stream_id) return;
        setMessages((prev) =>
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
          stateRef.current !== "expanded" &&
          stateRef.current !== "stealth"
        ) {
          const chatStr = getStrings(langRef.current).chat;
          presentBubble({
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
        const { stream_id, delta } = event.payload;
        if (activeStreamIdRef.current !== stream_id) return;
        setMessages((prev) =>
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
        const { stream_id, delta } = event.payload;
        if (activeStreamIdRef.current !== stream_id) return;
        const tw = typewriterRef.current;
        if (tw.streamId === stream_id) {
          tw.chars.push(...Array.from(delta));
        }
      })
    );

    unlisteners.push(
      listen<{ stream_id: string; full_text: string }>("buddy://ai-done", (event) => {
        if (activeStreamIdRef.current !== event.payload.stream_id) return;
        typewriterRef.current.streamDone = true;
        void refreshConversations();
        if (stateRef.current !== "expanded" && stateRef.current !== "stealth") {
          const chatStr = getStrings(langRef.current).chat;
          presentBubble({
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
        const { stream_id, error } = event.payload;
        if (activeStreamIdRef.current !== stream_id) return;
        activeStreamIdRef.current = null;
        typewriterRef.current.streamId = null;
        thinkingRef.current = false;
        setIsThinking(false);
        setActiveStreamId(null);
        if (stateRef.current === "expanded") {
          applyVisualTargets("expanded", false);
        } else if (stateRef.current !== "stealth") {
          const chatStr = getStrings(langRef.current).chat;
          presentBubble({
            kind: "reply_notice",
            tone: "error",
            durationMs: 5_000,
            text:
              chatStr.replyErrorPrefix +
              formatBubblePreview(error, chatStr.replyDoneFallback, 74),
          });
        }
        setMessages((prev) =>
          prev.map((m) =>
            m.id.toString() === stream_id ? { ...m, text: error, isError: true } : m
          )
        );
      })
    );

    unlisteners.push(
      listen("buddy://capture", () => triggerCapture()),
      listen("buddy://simulate-capture", () => triggerCapture()),
      listen("buddy://memory-cleaned", () => setMemoryRefresh((n) => n + 1)),
      listen("buddy://expand", () => transitionTo("expanded")),
      listen("buddy://collapse", () => {
        if (stateRef.current === "expanded") transitionTo(restStateOrMusic());
      }),
      listen<boolean>("buddy://toggle-panel", (e) => {
        if (e.payload) {
          activeTabRef.current = "chat";
          setActiveTab("chat");
          transitionTo("expanded");
          window.setTimeout(() => {
            window.dispatchEvent(new Event("buddy-focus-chat-input"));
          }, 45);
        } else if (stateRef.current === "expanded") {
          transitionTo(restStateOrMusic());
        }
      }),
      listen<boolean>("buddy://auto-hide-changed", (e) => {
        setSettings((prev) => ({ ...prev, auto_hide_stealth: e.payload }));
      }),
      listen<boolean>("buddy://music-reactive-changed", (e) => {
        setSettings((prev) => ({ ...prev, music_reactive: e.payload }));
      }),
      listen<DockStatePayload>("buddy://dock-changed", (e) => {
        const edge = (e.payload.dock_edge || "top") as DockEdge;
        dockEdgeRef.current = edge;
        setDockEdge(edge);
        snapPreviewRef.current = null;
        setSnapPreview(null);
        applyVisualTargets(stateRef.current);
      }),
      listen<UpdateInfo>("buddy://update-available", (e) => {
        setAvailableUpdate(e.payload);
      }),
      listen<{ downloaded: number; total?: number | null }>("buddy://update-progress", (e) => {
        const total = e.payload.total ?? 0;
        if (total > 0) {
          setUpdateProgress(Math.min(100, Math.round((e.payload.downloaded / total) * 100)));
        } else {
          setUpdateProgress(0);
        }
      }),
      listen<PluginProgressPayload>("buddy://plugin-progress", (e) => {
        setPluginProgress((prev) => ({ ...prev, [e.payload.plugin_id]: e.payload }));
      }),
      listen("buddy://skills-updated", () => {
        void invoke<SkillInfo[]>("list_skills").then(setSkills).catch(() => {});
      }),
      listen<ColorPickedPayload>("buddy://color-picked", (e) => {
        setLastPickedColor(e.payload);
        activeTabRef.current = "plugins";
        setActiveTab("plugins");
        setFocusedPluginId("color_picker");
        if (stateRef.current !== "expanded") {
          transitionTo("expanded");
        }
      }),
      listen<TimerTickPayload>("buddy://timer-tick", (e) => {
        setTimerState(e.payload.active ? e.payload : null);
      }),
      listen<string>("buddy://timer-done", () => {
        setTimerState(null);
        renderEyeSvg("happy", 12);
        springRef.current.vCapScale = 2.2;
        window.setTimeout(() => {
          if (stateRef.current !== "expanded") {
            applyVisualTargets(stateRef.current);
          }
        }, 1600);
      }),
      listen<ShareStatePayload>("buddy://share-state", (e) => {
        const wasActive = Boolean(shareStateRef.current?.active);
        const nextShare = e.payload;
        shareStateRef.current = nextShare;
        setShareState(nextShare);
        if (!nextShare.active) {
          if (floatingBubbleRef.current?.kind === "share_whisper") {
            dismissBubbleAnimated();
          }
        } else if (stateRef.current === "sleep") {
          transitionTo("idle");
        }
        if (wasActive !== nextShare.active && stateRef.current !== "expanded") {
          applyVisualTargets(stateRef.current);
        }
      }),
      listen<ShareWhisperPayload>("buddy://share-whisper", (e) => {
        const w = e.payload;
        if (stateRef.current === "expanded") {
          const cid = activeConversationIdRef.current;
          if (cid !== null) {
            void invoke("accept_share_whisper", { conversationId: cid, text: w.text }).catch(
              () => {}
            );
          }
          setMessages((prev) => [
            ...prev,
            { id: Date.now(), role: "assistant", text: w.text },
          ]);
        } else {
          presentBubble({
            kind: "share_whisper",
            whisper: w,
            durationMs: 14_000,
          });
        }
      })
    );

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && stateRef.current === "expanded") {
        transitionTo(restStateOrMusic());
      }
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      window.removeEventListener("keydown", onKeyDown);
      unlisteners.forEach((p) => p.then((un) => un()));
    };
  }, []);

  // 180Hz-ready GPU spring physics loop
  useEffect(() => {
    let rafId = 0;
    let lastTime = performance.now();
    let lastSleepDraw = 0;
    let frameCount = 0;
    let fpsAcc = 0;

    const drainTypewriter = (now: number) => {
      const tw = typewriterRef.current;
      if (tw.streamId && now - tw.lastTick >= 34) {
        const twDt = Math.min(0.1, (now - tw.lastTick) / 1000);
        tw.lastTick = now;
        const backlog = tw.chars.length - tw.shownLen;
        if (backlog > 0) {
          const cps = Math.min(85, 24 + backlog * 0.45);
          tw.acc += cps * twDt;
          const step = Math.min(backlog, Math.floor(tw.acc));
          if (step > 0) {
            tw.acc -= step;
            tw.shownLen += step;
            const visibleText = tw.chars.slice(0, tw.shownLen).join("");
            const targetId = tw.streamId;
            setMessages((prev) =>
              prev.map((m) =>
                m.id.toString() === targetId ? { ...m, text: visibleText } : m
              )
            );
          }
        } else if (tw.streamDone) {
          activeStreamIdRef.current = null;
          tw.streamId = null;
          thinkingRef.current = false;
          setIsThinking(false);
          setActiveStreamId(null);
          if (stateRef.current === "expanded") {
            applyVisualTargets("expanded", false);
          }
        }
      }
    };

    const tick = (now: number) => {
      const st = stateRef.current;
      const s = springRef.current;
      drainTypewriter(now);

      if (st === "stealth" && document.hidden) {
        lastTime = now;
        rafId = requestAnimationFrame(tick);
        return;
      }

      // Only throttle Sleep rendering AFTER shrink spring & eye morph have smoothly settled
      const emCur = eyeMorphRef.current.cur;
      const emTgt = eyeMorphRef.current.target;
      const sleepSettled =
        st === "sleep" &&
        Math.abs(s.w - s.tw) < 0.4 &&
        Math.abs(s.h - s.th) < 0.4 &&
        Math.abs(s.capScale - s.tCapScale) < 0.005 &&
        Math.abs(emCur.arcOp - emTgt.arcOp) < 0.02 &&
        Math.abs(emCur.pillOp - emTgt.pillOp) < 0.02;

      if (sleepSettled && now - lastSleepDraw < 110) {
        rafId = requestAnimationFrame(tick);
        return;
      }
      lastSleepDraw = now;

      const rawDt = (now - lastTime) / 1000;
      lastTime = now;
      const dt = Math.min(rawDt, 0.025);

      frameCount++;
      fpsAcc += rawDt;
      if (fpsAcc >= 0.75) {
        if (st === "expanded" && activeTabRef.current === "settings") {
          setRefreshHz(Math.round(frameCount / fpsAcc));
        }
        frameCount = 0;
        fpsAcc = 0;
      }

      const t = now / 1000;
      const f = animFlagsRef.current;

      const stepMusicOverlay = (isExpandedCtx: boolean) => {
        const m = musicRef.current;
        m.beatImpulse = Math.max(0, m.beatImpulse - dt * 5.2);
        m.phase += dt * (m.style === "lofi" ? 2.1 : m.style === "edm" ? 4.4 : 3.1);

        const barCap = isExpandedCtx ? 28 : 20;
        const targets = [
          (isExpandedCtx ? 6 : 4) + (m.bass * 0.85 + Math.abs(Math.sin(m.phase)) * 0.15) * (isExpandedCtx ? 20 : 15),
          (isExpandedCtx ? 6 : 4) + (m.mid * 0.8 + m.bass * 0.2) * (isExpandedCtx ? 24 : 17),
          (isExpandedCtx ? 6 : 4) +
            (m.mid * 0.65 + m.high * 0.35 + Math.abs(Math.cos(m.phase * 1.3)) * 0.12) *
              (isExpandedCtx ? 22 : 16),
          (isExpandedCtx ? 6 : 4) + (m.high * 0.85 + Math.abs(Math.sin(m.phase * 1.7)) * 0.15) * (isExpandedCtx ? 18 : 13),
        ];
        const barAlpha = Math.min(1, dt * 16);
        eqBarRefs.current.forEach((bar, i) => {
          if (!bar) return;
          const prevH = m.eqHeights[i] ?? 4;
          const nextH = prevH + (Math.min(barCap, targets[i] ?? 4) - prevH) * barAlpha;
          m.eqHeights[i] = nextH;
          bar.style.height = `${nextH.toFixed(1)}px`;
          bar.style.width = isExpandedCtx ? "3.5px" : "3px";
        });

        if (musicNoteRef.current) {
          const noteLift = Math.sin(m.phase * 0.9) * (isExpandedCtx ? 3.2 : 1.8) - m.beatImpulse * (isExpandedCtx ? 2.6 : 1.5);
          const noteTilt = Math.sin(m.phase * 0.9) * (isExpandedCtx ? 7.0 : 4.5);
          musicNoteRef.current.style.transform = `translateY(calc(-50% + ${noteLift.toFixed(1)}px)) rotate(${noteTilt.toFixed(1)}deg)`;
          musicNoteRef.current.style.opacity = (0.76 + Math.sin(m.phase) * 0.2).toFixed(2);
        }

        return Math.max(m.beatImpulse, m.bass * 0.72);
      };

      const stepWakeProgress = (): number => {
        if (f.wakeKind === "none") return 1;
        f.wakeElapsed += dt;
        const p = Math.min(1, f.wakeElapsed / f.wakeDuration);
        if (f.wakeKind === "from_sleep") {
          if (p < 0.42) {
            // Stage 1: drowsy squint uncurling from sleep curve
            eyeMorphRef.current.target = { ...EYE_POSES.doze, pillH: 8.5 };
            f.eyeSpeed = 7.5;
          } else {
            // Stage 2: gradually opening eyes wide
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 5.2;
          }
        } else {
          // Waking smoothly from half-closed doze
          if (p < 0.28) {
            eyeMorphRef.current.target = { ...EYE_POSES.doze, pillH: 14 };
            f.eyeSpeed = 6.5;
          } else {
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 5.8;
          }
        }
        if (p >= 1) {
          f.wakeKind = "none";
          f.eyeSpeed = 9.0;
        }
        return p;
      };

      if (st === "idle") {
        const liveShare = shareStateRef.current;
        if (liveShare?.active) {
          // Attentive watching gaze while sharing screen (subtle sweep + focused analyze glint)
          f.idleEyeMode = "open";
          if (liveShare.analyzing) {
            eyeMorphRef.current.target = { ...EYE_POSES.open, pillH: 8.2, glintOp: 0, glowOp: 0 };
            f.eyeSpeed = 12.0;
            s.tex = Math.sin(t * 2.8) * 3.0;
            s.tey = 1.2;
          } else {
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 7.5;
            s.tex = Math.sin(t * 0.75) * 4.2;
            s.tey = 1.6 + Math.sin(t * 0.42) * 0.8;
          }
          s.tCapScale = 1;
          s.tCapRot = 0;
        } else {
          f.idleElapsed += dt;
          if (f.idleElapsed >= f.idleDuration) {
            if (f.idleBehavior === "settle_drowsy") {
              f.idleBehavior = "long_doze";
              f.idleElapsed = 0;
              f.idleDuration = 8.0 + Math.random() * 5.0;
              f.idleEyeMode = "doze";
              renderEyeSvg("doze", 4.5);
              setCapsuleSizeTarget(152, 42, 21, 42);
            } else {
              pickNextIdleBehavior(f.idleBehavior);
            }
          }

          const el = f.idleElapsed;
          const dur = f.idleDuration;
          const kind = f.idleBehavior;

          if (kind === "settle_drowsy") {
            const p = Math.min(1, el / dur);
            s.tex = Math.sin(t * 0.8) * (1 - p) * 1.8;
            s.tey = p * 1.4;
            s.tCapScale = 1 + Math.sin(t * 1.4) * 0.01;
            s.tCapRot = 0;
            if (p > 0.42) {
              // Gradually let eyelids feel heavy and droop toward doze
              f.idleEyeMode = "doze";
              eyeMorphRef.current.target = { ...EYE_POSES.doze };
              f.eyeSpeed = 3.2;
            } else {
              f.idleEyeMode = "open";
              eyeMorphRef.current.target = { ...EYE_POSES.open };
              f.eyeSpeed = 6.0;
            }
          } else if (kind === "long_doze") {
          const nodCycle = (el * 0.45) % 1;
          const nodDrop =
            nodCycle > 0.78 ? Math.sin(((nodCycle - 0.78) / 0.22) * Math.PI) * 3.8 : 0;
          s.tex = Math.sin(t * 0.7) * 0.8;
          s.tey = 1.4 + nodDrop;
          s.tCapScale = 1 + Math.sin(t * 1.2) * 0.012;
          s.tCapRot = nodDrop > 1.5 ? f.idleDir * 1.4 : 0;
        } else if (kind === "startled_wake") {
          const p = el / dur;
          s.tex = p < 0.35 ? -4.2 * f.idleDir : p < 0.7 ? 4.2 * f.idleDir : 0;
          s.tey = -1.2;
          s.tCapScale = p < 0.25 ? 1.04 : 1.0;
          s.tCapRot = 0;
        } else if (kind === "daydream_dots") {
          s.tex = -5.0;
          s.tey = -2.4;
          s.tCapScale = 1;
          s.tCapRot = -1.5;
          if (idleDotsRef.current) {
            idleDotsRef.current.textContent = ".".repeat(1 + Math.floor((t * 2.4) % 3));
          }
        } else if (kind === "eavesdrop") {
          s.tex = 4.6 * f.idleDir;
          s.tey = 3.2;
          s.tCapRot = 3.2 * f.idleDir;
          s.tCapScale = 1.015;
        } else if (kind === "yawn") {
          // Asymmetric natural yawn curve: smooth ramp up -> peak hold -> gentle relax
          const p = Math.min(1, el / dur);
          const curve =
            p < 0.35
              ? Math.sin((p / 0.35) * (Math.PI / 2))
              : p < 0.65
              ? 1.0 - Math.sin(((p - 0.35) / 0.3) * Math.PI) * 0.08
              : Math.cos(((p - 0.65) / 0.35) * (Math.PI / 2));
          const eased = Math.max(0, curve);
          // Cute small vertical-oval "o" mouth fading smoothly in & out
          const rx = 1.5 + eased * 4.0;
          const ry = 1.5 + eased * 6.0;
          if (mouthShapeRef.current) {
            mouthShapeRef.current.innerHTML = `<ellipse cx="18" cy="12" rx="${rx.toFixed(1)}" ry="${ry.toFixed(1)}" fill="#ffffff" opacity="${(eased * 0.92).toFixed(2)}" />`;
          }
          s.tex = 0;
          s.tey = -7.5 * eased;
          s.tesy = 1 - eased * 0.45;
          s.tesx = 1 + eased * 0.1;
          s.tCapScale = 1 + eased * 0.03;
          s.tCapRot = -1.2 * eased;
        } else if (kind === "look_around") {
          const p = el / dur;
          s.tex = p < 0.33 ? -5.5 * f.idleDir : p < 0.66 ? 5.5 * f.idleDir : 0;
          s.tey = Math.sin(p * Math.PI * 2) * 1.4;
          s.tCapRot = s.tex * 0.35;
          s.tCapScale = 1;
        } else if (kind === "wink_play") {
          const p = Math.min(1, el / dur);
          const w = p > 0.2 && p < 0.75 ? Math.sin(((p - 0.2) / 0.55) * Math.PI) : 0;
          if (leftEyeShapeRef.current) {
            leftEyeShapeRef.current.style.transform = `scaleY(${Math.max(0.1, 1 - w * 0.9).toFixed(2)})`;
          }
          s.tex = 1.8 * f.idleDir;
          s.tey = -0.5;
          s.tCapRot = 2.6 * f.idleDir * w;
          s.tCapScale = 1 + w * 0.025;
        }
        }
      } else if (st === "sleep") {
        s.tex = 0;
        s.tey = Math.sin(t * 1.2) * 1.4;
        s.tCapScale = 1;
        if (sleepZRef.current) {
          sleepZRef.current.innerHTML = `<span style="opacity:${(0.3 + 0.45 * Math.sin(t * 2.2)).toFixed(2)}">z</span><span style="opacity:${(0.3 + 0.45 * Math.sin(t * 2.2 - 1.1)).toFixed(2)}; font-size:9px; margin-left:1px">Z</span>`;
        }
      } else if (st === "capture") {
        f.captureElapsed += dt;
        if (f.captureElapsed < 0.48) {
          const p = Math.min(1, f.captureElapsed / 0.48);
          const easeIn = p * p * (3 - 2 * p);
          const openAmt = Math.sin(p * Math.PI);
          const rx = 2.5 + openAmt * 3.8;
          const ry = 2.5 + openAmt * 4.4;
          if (mouthShapeRef.current) {
            mouthShapeRef.current.innerHTML = `<ellipse cx="18" cy="11" rx="${rx.toFixed(1)}" ry="${ry.toFixed(1)}" fill="#ffffff" opacity="${(0.65 + openAmt * 0.35).toFixed(2)}" />`;
          }
          const inv = 1 - easeIn;
          const swirl = Math.sin(p * Math.PI * 1.5) * 6 * inv;
          const cx = (facePivotRef.current?.clientWidth || 176) / 2;
          const offsets: [number, number][] = [
            [cx - 3 - inv * 54, 38 - inv * 12 + swirl],
            [cx + inv * 54, 38 - inv * 10 - swirl],
            [cx - 2 - inv * 32, 42 + inv * 18],
            [cx + 3 + inv * 34, 42 + inv * 16],
            [cx + swirl * 0.4, 40 + inv * 22],
          ];
          offsets.forEach(([ox, oy], idx) => {
            const el = particleElsRef.current[idx];
            if (el) {
              el.style.left = `${ox.toFixed(1)}px`;
              el.style.top = `${oy.toFixed(1)}px`;
              el.style.opacity = `${(inv * 0.92).toFixed(2)}`;
            }
          });
          s.tex = 0;
          s.tey = -5 * openAmt;
          s.tesx = 1.05;
          s.tesy = 1.05;
          s.tCapRot = 0;
        } else if (f.capturePhase === "inhale") {
          f.capturePhase = "swallow";
          particlesRef.current?.classList.add("hidden");
          mouthSvgRef.current?.classList.add("hidden");
          renderEyeSvg("happy", 14.0);
          if (!isSideDock(dockEdgeRef.current)) {
            captureToastRef.current?.classList.remove("hidden");
            captureToastRef.current?.classList.add("flex");
            setCapsuleSizeTarget(258, 44, 22, 44, false);
          } else {
            setCapsuleSizeTarget(176, 42, 21, 42, false);
          }
          s.tey = 0;
          s.vCapScale = -1.8;
          s.tCapRot = 0;
          s.tesx = 1;
          s.tesy = 1;
        }
      } else if (st === "peek") {
        const dock = dockEdgeRef.current;
        const rawDx = peekTargetRef.current.dx;
        const rawDy = peekTargetRef.current.dy;
        const localDx = dock === "left" ? rawDy : dock === "right" ? -rawDy : rawDx;
        const localDy = dock === "left" ? -rawDx : dock === "right" ? rawDx : rawDy;

        if (musicRef.current.playing) {
          stepWakeProgress();
          const punch = stepMusicOverlay(false);
          const m = musicRef.current;
          s.tex = localDx * 0.85 + Math.sin(m.phase) * 1.8;
          s.tey = localDy * 0.8 + punch * 2.0;
          s.teRot = Math.sin(m.phase) * 3.5;
          s.tCapRot = rawDx * 0.3;
          s.tCapScale = 1 + punch * 0.014;
        } else {
          const wakeP = stepWakeProgress();
          const gazeBlend = 0.25 + wakeP * 0.75;
          s.tex = localDx * gazeBlend;
          s.tey = localDy * gazeBlend;
          s.teRot = 0;
          s.tCapRot =
            wakeP < 0.5 && f.wakeKind === "from_sleep"
              ? -1.6 * (1 - wakeP * 2)
              : rawDx * 0.42;
        }
      } else if (st === "music") {
        stepWakeProgress();
        const m = musicRef.current;
        const punch = stepMusicOverlay(false);
        // Eyes sway cheerfully to the beat inside a steady capsule
        s.tex = Math.sin(m.phase) * 3.2;
        s.tey = punch * 2.6 + Math.sin(m.phase * 2.0) * 0.8;
        s.teRot = Math.sin(m.phase) * 4.2;
        s.tesx = 1;
        s.tesy = 1;
        s.tCapScale = 1 + punch * 0.015;
        s.tCapRot = 0;
      } else if (st === "stealth") {
        s.tex = 0;
        s.tey = 0;
        s.tCapRot = 0;
        s.tCapScale = 1;
      } else if (st === "expanded") {
        const m = musicRef.current;
        const punch = m.playing ? stepMusicOverlay(true) : 0;
        // Never sway the dialog box itself while expanded
        s.tCapRot = 0;
        s.tCapScale = 1;

        if (thinkingRef.current) {
          s.teRot = 0;
          const tl = thinkingLevelRef.current;
          if (tl === "off") {
            // Instant direct alert gaze (no sluggish deliberation)
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 14.0;
            s.tex = 0;
            s.tey = 0.8;
            idleDotsRef.current?.classList.add("hidden");
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = "";
              rightEyeShapeRef.current.style.transform = "";
              leftEyeShapeRef.current.style.opacity = "1";
              rightEyeShapeRef.current.style.opacity = "1";
            }
          } else if (tl === "low") {
            // Light quick check: calm glance & gentle shallow pulse
            eyeMorphRef.current.target = { ...EYE_POSES.open, glowOp: 0.08 };
            f.eyeSpeed = 10.0;
            s.tex = Math.sin(t * 1.1) * 1.8;
            s.tey = -1.2;
            idleDotsRef.current?.classList.add("hidden");
            const breath = 0.5 + 0.5 * Math.sin(t * 2.8);
            const scalePulse = 0.97 + breath * 0.06;
            const opacityPulse = 0.82 + breath * 0.18;
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              rightEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              leftEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
              rightEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
            }
          } else if (tl === "high") {
            // Deep focused analysis: slightly narrowed eyes weighing pros & cons
            eyeMorphRef.current.target = { ...EYE_POSES.open, pillW: 27, pillH: 27, glowOp: 0.3 };
            f.eyeSpeed = 9.0;
            s.tex = Math.sin(t * 2.3) * 4.4;
            s.tey = -1.4;
            s.teRot = Math.sin(t * 1.15) * 1.6;
            const breath = 0.5 + 0.5 * Math.sin(t * 5.2);
            idleDotsRef.current?.classList.add("hidden");
            const scalePulse = 0.91 + breath * 0.16;
            const opacityPulse = 0.58 + breath * 0.42;
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              rightEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              leftEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
              rightEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
            }
          } else if (tl === "max") {
            // Exhaustive multi-branch simulation: 4-corner scenario scan + focus dots + deep pulse
            eyeMorphRef.current.target = { ...EYE_POSES.open, pillW: 28, pillH: 24, glowOp: 0.48 };
            f.eyeSpeed = 9.0;
            const stepIdx = Math.floor(t / 0.85) % 4;
            const branchTargets: [number, number][] = [
              [-4.8, -3.2],
              [4.8, -3.2],
              [-2.2, 1.6],
              [2.2, -2.0],
            ];
            s.tex = branchTargets[stepIdx][0];
            s.tey = branchTargets[stepIdx][1];
            s.teRot = Math.sin(t * 1.8) * 2.2;
            const breath = 0.5 + 0.5 * Math.sin(t * 6.0);
            if (idleDotsRef.current) {
              idleDotsRef.current.classList.remove("hidden");
              idleDotsRef.current.style.top = "26px";
              idleDotsRef.current.style.transform = "translateX(56px)";
              idleDotsRef.current.textContent = ".".repeat(1 + (Math.floor(t * 3.0) % 3));
            }
            const scalePulse = 0.9 + breath * 0.18;
            const opacityPulse = 0.54 + breath * 0.46;
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              rightEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              leftEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
              rightEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
            }
          } else {
            // Medium: Balanced pure-white breathing pulse with smooth curious gaze
            eyeMorphRef.current.target = { ...EYE_POSES.thinking };
            f.eyeSpeed = 10.0;
            s.tex = Math.sin(t * 1.6) * 2.8;
            s.tey = -2.0 + Math.cos(t * 2.2) * 1.0;
            idleDotsRef.current?.classList.add("hidden");
            const breath = 0.5 + 0.5 * Math.sin(t * 4.5);
            const scalePulse = 0.94 + breath * 0.12;
            const opacityPulse = 0.62 + breath * 0.38;
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              rightEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              leftEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
              rightEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
            }
          }
        } else {
          idleDotsRef.current?.classList.add("hidden");
          if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
            leftEyeShapeRef.current.style.transform = "";
            rightEyeShapeRef.current.style.transform = "";
            leftEyeShapeRef.current.style.opacity = "1";
            rightEyeShapeRef.current.style.opacity = "1";
          }
          stepWakeProgress();
          if (focusedRef.current) {
            s.tex = Math.sin(t * 2.4) * 4.2;
            s.tey = 8.5 + (m.playing ? punch * 1.6 : 0);
            s.teRot = m.playing ? Math.sin(m.phase) * 2.2 : 0;
          } else if (m.playing) {
            // Buddy's eyes sway cheerfully inside a rock-steady dialog box!
            s.tex = Math.sin(m.phase) * 4.8;
            s.tey = punch * 3.0 + Math.cos(m.phase * 2.0) * 1.0;
            s.teRot = Math.sin(m.phase) * 4.8;
            s.tesx = 1;
            s.tesy = 1;
          } else {
            s.teRot = 0;
            s.tex = Math.sin(t * 0.7) * 2.4;
            s.tey = Math.cos(t * 0.5) * 1.2;
            f.winkTimer += dt;
            if (f.winkTimer > 6.5) {
              f.winkProgress += dt * 5.5;
              const w = Math.sin(f.winkProgress * Math.PI);
              if (leftEyeShapeRef.current) {
                leftEyeShapeRef.current.style.transform = `scaleY(${Math.max(0.1, 1 - w * 0.9).toFixed(2)})`;
              }
              if (f.winkProgress >= 1) {
                f.winkProgress = 0;
                f.winkTimer = 0;
                if (leftEyeShapeRef.current) leftEyeShapeRef.current.style.transform = "";
              }
            }
          }
        }
      }

      if (
        !(st === "idle" && f.idleBehavior === "wink_play") &&
        !(st === "expanded" && (thinkingRef.current || f.winkProgress > 0))
      ) {
        if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
          leftEyeShapeRef.current.style.transform = "";
          rightEyeShapeRef.current.style.transform = "";
          leftEyeShapeRef.current.style.opacity = "1";
          rightEyeShapeRef.current.style.opacity = "1";
        }
      }

      // Smooth parametric SVG eye morphing (exponential damping at VSync speed)
      const em = eyeMorphRef.current;
      const eAlpha = Math.min(1, dt * f.eyeSpeed);
      em.cur.pillW += (em.target.pillW - em.cur.pillW) * eAlpha;
      em.cur.pillH += (em.target.pillH - em.cur.pillH) * eAlpha;
      em.cur.pillOp += (em.target.pillOp - em.cur.pillOp) * eAlpha;
      em.cur.glintOp += (em.target.glintOp - em.cur.glintOp) * eAlpha;
      em.cur.arcBend += (em.target.arcBend - em.cur.arcBend) * eAlpha;
      em.cur.arcOp += (em.target.arcOp - em.cur.arcOp) * eAlpha;
      em.cur.glowOp += (em.target.glowOp - em.cur.glowOp) * eAlpha;
      em.cur.isHappyGreen = em.target.isHappyGreen;
      applyEyePoseAttributes(em.cur);

      if (
        f.wakeKind === "none" &&
        ((st === "expanded" && !thinkingRef.current) ||
          st === "peek" ||
          st === "music" ||
          (st === "idle" &&
            animFlagsRef.current.idleEyeMode === "open" &&
            animFlagsRef.current.idleBehavior !== "wink_play"))
      ) {
        f.blinkTimer += dt;
        if (f.blinkTimer >= f.nextBlinkAt) {
          f.blinkProgress = 0.001;
          f.blinkTimer = 0;
          f.nextBlinkAt = 2.6 + Math.random() * 2.8;
        }
        if (f.blinkProgress > 0) {
          f.blinkProgress += dt * 7.5;
          const b = Math.sin(f.blinkProgress * Math.PI);
          s.tesy = Math.max(0.08, 1 - b * 0.92);
          s.tesx = 1 + b * 0.12;
          if (f.blinkProgress >= 1) {
            f.blinkProgress = 0;
            s.tesy = 1;
            s.tesx = 1;
          }
        }
      }

      if (isDraggingRef.current && snapPreviewRef.current) {
        const sp = snapPreviewRef.current;
        s.tex = sp === "left" ? -5.5 : sp === "right" ? 5.5 : 0;
        s.tey = sp === "top" ? -3.5 : sp === "bottom" ? 3.5 : 0;
      }

      [s.w, s.vw] = stepSpring(s.w, s.vw, s.tw, 420, 34, dt);
      [s.h, s.vh] = stepSpring(s.h, s.vh, s.th, 420, 34, dt);
      [s.r, s.vr] = stepSpring(s.r, s.vr, s.tr, 420, 34, dt);
      [s.stageH, s.vStageH] = stepSpring(s.stageH, s.vStageH, s.tStageH, 420, 34, dt);
      [s.stageW, s.vStageW] = stepSpring(s.stageW, s.vStageW, s.tStageW, 420, 34, dt);
      [s.headRot, s.vHeadRot] = stepSpring(s.headRot, s.vHeadRot, s.tHeadRot, 340, 26, dt);
      [s.ex, s.vex] = stepSpring(s.ex, s.vex, s.tex, 260, 22, dt);
      [s.ey, s.vey] = stepSpring(s.ey, s.vey, s.tey, 260, 22, dt);
      [s.eRot, s.veRot] = stepSpring(s.eRot, s.veRot, s.teRot, 240, 20, dt);
      [s.esx, s.vesx] = stepSpring(s.esx, s.vesx, s.tesx, 380, 24, dt);
      [s.esy, s.vesy] = stepSpring(s.esy, s.vesy, s.tesy, 380, 24, dt);
      [s.capScale, s.vCapScale] = stepSpring(s.capScale, s.vCapScale, s.tCapScale, 340, 22, dt);
      [s.capY, s.vCapY] = stepSpring(s.capY, s.vCapY, s.tCapY, 240, 24, dt);
      [s.capRot, s.vCapRot] = stepSpring(s.capRot, s.vCapRot, s.tCapRot, 260, 22, dt);
      [s.eyeSize, s.vEyeSize] = stepSpring(s.eyeSize, s.vEyeSize, s.tEyeSize, 400, 32, dt);

      const dims = layoutDimsRef.current;
      const dock = dockEdgeRef.current;
      const side = isSideDock(dock);
      const clampedW = Math.min(dims.canvas_w - dims.pad * 2, Math.max(36, s.w));
      const clampedH = Math.min(dims.canvas_h - dims.pad * 2, Math.max(36, s.h));

      let boxX = (dims.canvas_w - clampedW) / 2;
      let boxY = dims.pad;
      if (dock === "left") {
        boxX = dims.pad;
        boxY = (dims.canvas_h - clampedH) / 2;
      } else if (dock === "right") {
        boxX = dims.canvas_w - dims.pad - clampedW;
        boxY = (dims.canvas_h - clampedH) / 2;
      } else if (dock === "bottom") {
        boxX = (dims.canvas_w - clampedW) / 2;
        boxY = dims.canvas_h - dims.pad - clampedH;
      }

      const rad = Math.min(s.r, clampedW / 2, clampedH / 2);

      if (pillRectRef.current && pillShadowRef.current) {
        const wStr = clampedW.toFixed(1);
        const hStr = clampedH.toFixed(1);
        const rStr = rad.toFixed(1);
        const xStr = boxX.toFixed(1);
        const yStr = boxY.toFixed(1);

        pillRectRef.current.setAttribute("x", xStr);
        pillRectRef.current.setAttribute("y", yStr);
        pillRectRef.current.setAttribute("width", wStr);
        pillRectRef.current.setAttribute("height", hStr);
        pillRectRef.current.setAttribute("rx", rStr);

        pillShadowRef.current.setAttribute("x", (boxX - 1.5).toFixed(1));
        pillShadowRef.current.setAttribute("y", (boxY + 1).toFixed(1));
        pillShadowRef.current.setAttribute("width", (clampedW + 3).toFixed(1));
        pillShadowRef.current.setAttribute("height", (clampedH + 3).toFixed(1));
        pillShadowRef.current.setAttribute("rx", (rad + 1.5).toFixed(1));
      }

      if (clipBoxRef.current) {
        const insetTop = Math.max(0, boxY).toFixed(1);
        const insetLeft = Math.max(0, boxX).toFixed(1);
        const insetRight = Math.max(0, dims.canvas_w - boxX - clampedW).toFixed(1);
        const insetBottom = Math.max(0, dims.canvas_h - boxY - clampedH).toFixed(1);
        clipBoxRef.current.style.clipPath = `inset(${insetTop}px ${insetRight}px ${insetBottom}px ${insetLeft}px round ${rad.toFixed(1)}px)`;
      }

      if (morphLayerRef.current) {
        morphLayerRef.current.style.transform =
          st === "expanded"
            ? ""
            : `translateY(${s.capY.toFixed(2)}px) rotate(${s.capRot.toFixed(2)}deg) scale(${s.capScale.toFixed(4)})`;
      }

      if (eyesStageRef.current) {
        if (side) {
          eyesStageRef.current.style.width = `${s.stageW.toFixed(1)}px`;
          eyesStageRef.current.style.height = "100%";
        } else {
          eyesStageRef.current.style.width = "100%";
          eyesStageRef.current.style.height = `${s.stageH.toFixed(1)}px`;
        }
      }

      if (facePivotRef.current) {
        if (side) {
          facePivotRef.current.style.width = "220px";
          facePivotRef.current.style.height = "64px";
          facePivotRef.current.style.position = "absolute";
          facePivotRef.current.style.left = "50%";
          facePivotRef.current.style.top = "50%";
          facePivotRef.current.style.transform = `translate(-50%, -50%) rotate(${s.headRot.toFixed(2)}deg)`;
        } else {
          facePivotRef.current.style.width = "100%";
          facePivotRef.current.style.height = "100%";
          facePivotRef.current.style.position = "relative";
          facePivotRef.current.style.left = "0";
          facePivotRef.current.style.top = "0";
          facePivotRef.current.style.transform = `rotate(${s.headRot.toFixed(2)}deg)`;
        }
      }

      if (eyesPairRef.current) {
        const eyeScale = s.eyeSize / 44.0;
        eyesPairRef.current.style.transform = `translate(${s.ex.toFixed(2)}px, ${s.ey.toFixed(2)}px) rotate(${s.eRot.toFixed(2)}deg) scale(${(eyeScale * s.esx).toFixed(3)}, ${(eyeScale * s.esy).toFixed(3)})`;
      }

      rafId = requestAnimationFrame(tick);
    };

    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, []);

  useLayoutEffect(() => {
    const s = springRef.current;
    const dims = layoutDims;
    const dock = dockEdge;
    const side = isSideDock(dock);
    const cw = Math.min(dims.canvas_w - dims.pad * 2, Math.max(36, s.w));
    const ch = Math.min(dims.canvas_h - dims.pad * 2, Math.max(36, s.h));
    let bx = (dims.canvas_w - cw) / 2;
    let by = dims.pad;
    if (dock === "left") {
      bx = dims.pad;
      by = (dims.canvas_h - ch) / 2;
    } else if (dock === "right") {
      bx = dims.canvas_w - dims.pad - cw;
      by = (dims.canvas_h - ch) / 2;
    } else if (dock === "bottom") {
      bx = (dims.canvas_w - cw) / 2;
      by = dims.canvas_h - dims.pad - ch;
    }
    const rad = Math.min(s.r, cw / 2, ch / 2);
    if (pillRectRef.current && pillShadowRef.current) {
      pillRectRef.current.setAttribute("x", bx.toFixed(1));
      pillRectRef.current.setAttribute("y", by.toFixed(1));
      pillRectRef.current.setAttribute("width", cw.toFixed(1));
      pillRectRef.current.setAttribute("height", ch.toFixed(1));
      pillRectRef.current.setAttribute("rx", rad.toFixed(1));
      pillShadowRef.current.setAttribute("x", (bx - 1.5).toFixed(1));
      pillShadowRef.current.setAttribute("y", (by + 1).toFixed(1));
      pillShadowRef.current.setAttribute("width", (cw + 3).toFixed(1));
      pillShadowRef.current.setAttribute("height", (ch + 3).toFixed(1));
      pillShadowRef.current.setAttribute("rx", (rad + 1.5).toFixed(1));
    }
    if (clipBoxRef.current) {
      const top = Math.max(0, by).toFixed(1);
      const left = Math.max(0, bx).toFixed(1);
      const right = Math.max(0, dims.canvas_w - bx - cw).toFixed(1);
      const bottom = Math.max(0, dims.canvas_h - by - ch).toFixed(1);
      clipBoxRef.current.style.clipPath = `inset(${top}px ${right}px ${bottom}px ${left}px round ${rad.toFixed(1)}px)`;
    }
    if (eyesStageRef.current) {
      if (side) {
        eyesStageRef.current.style.width = `${s.stageW.toFixed(1)}px`;
        eyesStageRef.current.style.height = "100%";
      } else {
        eyesStageRef.current.style.width = "100%";
        eyesStageRef.current.style.height = `${s.stageH.toFixed(1)}px`;
      }
    }
  }, [layoutDims, dockEdge]);

  const handleSendChat = async (
    promptOverride?: string,
    attOverride?: AttachmentPayload[]
  ) => {
    const rawPrompt = (promptOverride ?? chatInput).trim();
    const usedAtts = attOverride ?? attachments;
    if ((!rawPrompt && usedAtts.length === 0) || isThinking) return;

    let convId = activeConversationIdRef.current;
    if (convId === null) {
      try {
        convId = await invoke<number>("ensure_active_conversation");
        activeConversationIdRef.current = convId;
        setActiveConversationId(convId);
      } catch {
        convId = 1;
      }
    }

    const finalPrompt = rawPrompt || "Analyze attached item";
    if (!promptOverride) setChatInput("");
    if (!attOverride) setAttachments([]);

    const userMsgId = Date.now();
    const streamId = (userMsgId + 1).toString();
    activeStreamIdRef.current = streamId;
    typewriterRef.current = {
      streamId,
      chars: [],
      shownLen: 0,
      streamDone: false,
      acc: 0,
      lastTick: performance.now(),
    };
    thinkingRef.current = true;
    setIsThinking(true);
    setActiveStreamId(streamId);
    applyVisualTargets("expanded", true);

    setMessages((prev) => [
      ...prev,
      { id: userMsgId, role: "user", text: finalPrompt, attachments: usedAtts },
      { id: userMsgId + 1, role: "assistant", text: "" },
    ]);

    void invoke("send_chat_message", {
      conversationId: convId,
      streamId,
      prompt: finalPrompt,
      attachments: usedAtts,
    })
      .then(() => refreshConversations())
      .catch((err) => {
        activeStreamIdRef.current = null;
        typewriterRef.current.streamId = null;
        thinkingRef.current = false;
        setIsThinking(false);
        setActiveStreamId(null);
        applyVisualTargets("expanded", false);
        setMessages((prev) =>
          prev.map((m) =>
            m.id.toString() === streamId ? { ...m, text: String(err), isError: true } : m
          )
        );
      });
  };

  const handleStopStream = () => {
    void invoke("cancel_chat_stream");
    activeStreamIdRef.current = null;
    typewriterRef.current.streamId = null;
    thinkingRef.current = false;
    setIsThinking(false);
    setActiveStreamId(null);
    applyVisualTargets("expanded", false);
  };

  const handleNewChat = async () => {
    if (isThinking) return;
    const curLang: UiLang = settings.ui_language === "zh" ? "zh" : "en";
    try {
      const created = await invoke<ConversationSummary>("create_conversation");
      activeConversationIdRef.current = created.id;
      setActiveConversationId(created.id);
      setMessages([makeWelcomeMessage(curLang)]);
      await refreshConversations();
    } catch {
      setMessages([makeWelcomeMessage(curLang)]);
    }
  };

  const handleToggleAgentMode = () => {
    if (isThinking) return;
    const next = { ...settings, agent_mode: !settings.agent_mode };
    setSettings(next);
    void invoke<AppSettings>("save_settings", {
      settings: next,
      newApiKey: null,
    })
      .then(setSettings)
      .catch(() => {});
  };

  const handleChangePermissionMode = (mode: AgentPermissionMode) => {
    setSettings((prev) => ({ ...prev, agent_permission_mode: mode }));
    void invoke<string>("set_agent_permission_mode", { mode })
      .then((normalized) => {
        setSettings((prev) => ({ ...prev, agent_permission_mode: normalized }));
      })
      .catch(() => {});
  };

  const handleChangeThinkingLevel = (level: ThinkingLevel) => {
    if (isThinking) return;
    const next = { ...settings, thinking_level: level };
    setSettings(next);
    void invoke<AppSettings>("save_settings", {
      settings: next,
      newApiKey: null,
    })
      .then(setSettings)
      .catch(() => {});
  };

  const handleResolveApproval = (stepId: string, decision: ApprovalDecision) => {
    void invoke("resolve_agent_approval", { stepId, decision }).catch(() => {});
  };

  const handleSettingsSaved = (updated: AppSettings) => {
    setSettings(updated);
    const nextLang: UiLang = updated.ui_language === "zh" ? "zh" : "en";
    setMessages((prev) =>
      prev.length === 1 && prev[0].id === 1 ? [makeWelcomeMessage(nextLang)] : prev
    );
  };

  useEffect(() => {
    if (whisperTimerRef.current) {
      window.clearTimeout(whisperTimerRef.current);
      whisperTimerRef.current = null;
    }

    if (buddyState === "expanded" && floatingBubble) {
      if (bubbleExitTimerRef.current) {
        window.clearTimeout(bubbleExitTimerRef.current);
        bubbleExitTimerRef.current = null;
      }
      setBubbleVisible(false);
      floatingBubbleRef.current = null;
      setFloatingBubble(null);
      setWhisperHovered(false);
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
      return;
    }

    if (!floatingBubble) {
      floatingBubbleRef.current = null;
      setBubbleVisible(false);
      setWhisperHovered(false);
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
      return;
    }

    const visible =
      buddyState !== "expanded" &&
      buddyState !== "stealth" &&
      (floatingBubble.kind !== "share_whisper" || Boolean(shareState?.active));

    if (visible && whisperRef.current) {
      const el = whisperRef.current;
      const bounds: [number, number, number, number] = [
        el.offsetLeft,
        el.offsetTop,
        el.offsetWidth,
        el.offsetHeight,
      ];
      void invoke("set_whisper_bounds", { bounds }).catch(() => {});
      if (!whisperHovered) {
        whisperTimerRef.current = window.setTimeout(() => {
          dismissBubbleAnimated();
        }, floatingBubble.durationMs);
      }
    } else {
      void invoke("set_whisper_bounds", { bounds: null }).catch(() => {});
    }

    return () => {
      if (whisperTimerRef.current) {
        window.clearTimeout(whisperTimerRef.current);
      }
    };
  }, [floatingBubble, shareState?.active, buddyState, dockEdge, layoutDims, whisperHovered]);

  const handleOpenBubble = async () => {
    if (!floatingBubble) return;
    const bubble = floatingBubble;
    if (bubbleExitTimerRef.current) {
      window.clearTimeout(bubbleExitTimerRef.current);
      bubbleExitTimerRef.current = null;
    }
    setBubbleVisible(false);
    floatingBubbleRef.current = null;
    setFloatingBubble(null);
    setWhisperHovered(false);
    if (bubble.kind === "reply_notice") {
      switchTab("chat");
      transitionTo("expanded");
      return;
    }
    const w = bubble.whisper;
    let convId = activeConversationIdRef.current;
    if (convId === null) {
      try {
        convId = await invoke<number>("ensure_active_conversation");
        activeConversationIdRef.current = convId;
        setActiveConversationId(convId);
      } catch {
        convId = 1;
      }
    }
    try {
      await invoke("accept_share_whisper", { conversationId: convId, text: w.text });
    } catch {
      // ignore
    }
    setMessages((prev) => [...prev, { id: Date.now(), role: "assistant", text: w.text }]);
    transitionTo("expanded");
  };

  const handleSelectConversation = (convId: number) => {
    if (isThinking) return;
    void loadConversationMessages(convId);
  };

  const handleDeleteConversation = async (convId: number) => {
    try {
      await invoke("delete_conversation", { conversationId: convId });
      if (activeConversationIdRef.current === convId) {
        const nextId = await invoke<number>("ensure_active_conversation");
        await loadConversationMessages(nextId);
      }
      await refreshConversations();
    } catch {
      // ignore
    }
  };

  const handleCheckUpdate = async (): Promise<{ info: UpdateInfo | null; error?: string }> => {
    try {
      const info = await invoke<UpdateInfo | null>("check_app_update");
      if (info) setAvailableUpdate(info);
      return { info };
    } catch (err) {
      return { info: null, error: String(err) };
    }
  };

  const handleInstallUpdate = () => {
    if (updateProgress !== null) return;
    setUpdateProgress(0);
    void invoke("install_app_update").catch(() => {
      setUpdateProgress(null);
    });
  };

  const handleResetDock = () => {
    void invoke<DockStatePayload>("reset_dock_position")
      .then((res) => {
        const edge = (res.dock_edge || "top") as DockEdge;
        dockEdgeRef.current = edge;
        setDockEdge(edge);
        snapPreviewRef.current = null;
        setSnapPreview(null);
        applyVisualTargets(stateRef.current);
      })
      .catch(() => {});
  };

  const handleDragPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("button, input, textarea, select, a")) return;
    // When collapsed, expand/drag ONLY if click lands directly inside Buddy's visible capsule body.
    if (stateRef.current !== "expanded" && pillRectRef.current) {
      const pr = pillRectRef.current.getBoundingClientRect();
      if (
        e.clientX < pr.left ||
        e.clientX > pr.right ||
        e.clientY < pr.top ||
        e.clientY > pr.bottom
      ) {
        return;
      }
    }
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }
    dragMetaRef.current = {
      active: true,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      moved: false,
    };
  };

  const handleDragPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const meta = dragMetaRef.current;
    if (!meta || !meta.active || meta.pointerId !== e.pointerId) return;

    if (!meta.moved) {
      const dist = Math.hypot(e.clientX - meta.startX, e.clientY - meta.startY);
      if (dist <= 5) return;
      meta.moved = true;
      isDraggingRef.current = true;
      void invoke("begin_buddy_drag");
      return;
    }

    if (dragStepInFlightRef.current) return;
    dragStepInFlightRef.current = true;
    void invoke<DockStatePayload>("step_buddy_drag")
      .then((res) => {
        const edge = (res.dock_edge || "free") as DockEdge;
        if (edge !== dockEdgeRef.current) {
          dockEdgeRef.current = edge;
          setDockEdge(edge);
          applyVisualTargets(stateRef.current);
        }
        const nextSnap = (res.snap_preview ?? null) as DockEdge | null;
        if (nextSnap !== snapPreviewRef.current) {
          snapPreviewRef.current = nextSnap;
          setSnapPreview(nextSnap);
        }
      })
      .catch(() => {})
      .finally(() => {
        dragStepInFlightRef.current = false;
      });
  };

  const handleDragPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    const meta = dragMetaRef.current;
    if (!meta || !meta.active || meta.pointerId !== e.pointerId) return;
    dragMetaRef.current = null;
    try {
      if (e.currentTarget.hasPointerCapture(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    } catch {
      // ignore
    }

    if (meta.moved) {
      isDraggingRef.current = false;
      void invoke<DockStatePayload>("end_buddy_drag")
        .then((res) => {
          const edge = (res.dock_edge || "top") as DockEdge;
          dockEdgeRef.current = edge;
          setDockEdge(edge);
          snapPreviewRef.current = null;
          setSnapPreview(null);
          applyVisualTargets(stateRef.current);
        })
        .catch(() => {});
    } else if (stateRef.current !== "expanded") {
      transitionTo("expanded");
    }
  };

  const isExpanded = buddyState === "expanded";
  const sideDocked = isSideDock(dockEdge);
  const enabledPluginsMap: Record<string, boolean> = {};
  const installedPluginsMap: Record<string, boolean> = {};
  for (const p of pluginsData.plugins) {
    enabledPluginsMap[p.id] = p.enabled;
    installedPluginsMap[p.id] = p.installed;
  }

  const handleQuickOcr = (source: string) => {
    void invoke<OcrResult>("plugin_offline_ocr", { source })
      .then((res) => {
        const txt = res.text.trim() || "(no text detected)";
        setMessages((prev) => [
          ...prev,
          {
            id: Date.now(),
            role: "assistant",
            text: `**Offline OCR (${res.language}):**\n\`\`\`\n${txt}\n\`\`\``,
          },
        ]);
        switchTab("chat");
      })
      .catch((err) => {
        setMessages((prev) => [
          ...prev,
          { id: Date.now(), role: "assistant", text: String(err), isError: true },
        ]);
      });
  };

  const handleQuickDownloadUrl = (url: string, format: string) => {
    setChatInput("");
    setMessages((prev) => [
      ...prev,
      { id: Date.now(), role: "user", text: `${url} (${format})` },
    ]);
    void invoke<PluginToolOutput>("plugin_download_media", {
      url,
      format,
      outputDir: null,
    })
      .then((res) => {
        setMessages((prev) => [
          ...prev,
          {
            id: Date.now() + 1,
            role: "assistant",
            text: `**${res.summary}**\n\`${res.outputs[0] ?? ""}\``,
            outputPaths: res.outputs,
          },
        ]);
      })
      .catch((err) => {
        setMessages((prev) => [
          ...prev,
          { id: Date.now() + 1, role: "assistant", text: String(err), isError: true },
        ]);
      });
  };
  const dims = layoutDims;
  const activeDims: [number, number] = sideDocked
    ? [dims.h_w, dims.h_h]
    : [dims.v_w, dims.v_h];
  const cardPosStyle: React.CSSProperties =
    dockEdge === "left"
      ? {
          left: dims.pad,
          top: (dims.canvas_h - dims.h_h) / 2,
          width: dims.h_w,
          height: dims.h_h,
          flexDirection: "row",
        }
      : dockEdge === "right"
      ? {
          left: dims.canvas_w - dims.pad - dims.h_w,
          top: (dims.canvas_h - dims.h_h) / 2,
          width: dims.h_w,
          height: dims.h_h,
          flexDirection: "row-reverse",
        }
      : dockEdge === "bottom"
      ? {
          left: (dims.canvas_w - dims.v_w) / 2,
          top: dims.canvas_h - dims.pad - dims.v_h,
          width: dims.v_w,
          height: dims.v_h,
          flexDirection: "column-reverse",
        }
      : {
          left: (dims.canvas_w - dims.v_w) / 2,
          top: dims.pad,
          width: dims.v_w,
          height: dims.v_h,
          flexDirection: "column",
        };

  const lang: UiLang = settings.ui_language === "zh" ? "zh" : "en";
  langRef.current = lang;
  const activeChatModel = resolveActiveChatModel(settings);
  const thinkingCapability = getModelThinkingCapability(activeChatModel);
  const thinkingLevel: ThinkingLevel = clampThinkingLevel(
    normalizeThinkingLevel(settings.thinking_level),
    thinkingCapability
  );
  thinkingLevelRef.current = thinkingLevel;
  const ui = getStrings(lang);

  return (
    <div className="w-full h-full bg-transparent relative overflow-hidden pointer-events-none">
      <div
        ref={morphLayerRef}
        className="relative pointer-events-none"
        style={{
          width: dims.canvas_w,
          height: dims.canvas_h,
          transformOrigin: "center center",
          willChange: isExpanded ? undefined : "transform",
        }}
      >
        <svg
          width={dims.canvas_w}
          height={dims.canvas_h}
          viewBox={`0 0 ${dims.canvas_w} ${dims.canvas_h}`}
          className="absolute top-0 left-0 pointer-events-none overflow-visible"
        >
          <rect
            ref={pillShadowRef}
            fill="rgba(0,0,0,0.34)"
          />
          <rect
            ref={pillRectRef}
            fill="#050507"
            stroke={snapPreview ? "rgba(56,189,248,0.65)" : "rgba(255,255,255,0.14)"}
            strokeWidth={snapPreview ? "1.6" : "1"}
          />
        </svg>

        {/* Magnetic Edge Snap Preview Glow */}
        {snapPreview && (
          <div
            className={`pointer-events-none absolute z-30 rounded-full bg-sky-400/90 shadow-[0_0_14px_rgba(56,189,248,0.85)] transition-opacity ${
              snapPreview === "left"
                ? "left-1 top-1/2 -translate-y-1/2 w-[3px] h-16"
                : snapPreview === "right"
                ? "right-1 top-1/2 -translate-y-1/2 w-[3px] h-16"
                : snapPreview === "bottom"
                ? "bottom-1 left-1/2 -translate-x-1/2 h-[3px] w-20"
                : "top-1 left-1/2 -translate-x-1/2 h-[3px] w-20"
            }`}
          />
        )}

        <div
          ref={clipBoxRef}
          onPointerDown={handleDragPointerDown}
          onPointerMove={handleDragPointerMove}
          onPointerUp={handleDragPointerUp}
          onPointerCancel={handleDragPointerUp}
          className={`relative w-full h-full pointer-events-auto ${
            isExpanded ? "cursor-default" : "cursor-grab active:cursor-grabbing"
          }`}
          style={{ willChange: "clip-path" }}
        >
          <div
            ref={cardFrameRef}
            className="absolute flex"
            style={cardPosStyle}
          >
            {/* Capsule Timer Luminous Edge Progress Bar */}
            {timerState?.active && timerState.total_sec > 0 && (
              <div className="pointer-events-none absolute inset-x-3 bottom-0.5 h-[2.5px] rounded-full bg-white/10 overflow-hidden z-30">
                <div
                  className="h-full bg-gradient-to-r from-sky-400 to-emerald-300 transition-all duration-500"
                  style={{
                    width: `${Math.max(2, Math.round((timerState.remaining_sec / timerState.total_sec) * 100))}%`,
                  }}
                />
              </div>
            )}
            {/* HERO EYES STAGE */}
            <div
              ref={eyesStageRef}
              className={`relative flex items-center justify-center shrink-0 overflow-hidden ${
                isExpanded ? "cursor-grab active:cursor-grabbing" : ""
              } ${
                isExpanded && dockEdge === "left"
                  ? "border-r border-white/[0.06]"
                  : isExpanded && dockEdge === "right"
                  ? "border-l border-white/[0.06]"
                  : isExpanded && dockEdge === "bottom"
                  ? "border-t border-white/[0.06]"
                  : ""
              }`}
            >
              <div
                ref={facePivotRef}
                className="relative flex items-center justify-center w-full h-full pointer-events-none"
                style={{ willChange: "transform" }}
              >
                <div
                  ref={eyesPairRef}
                  className="flex items-center justify-center gap-7"
                  style={{ willChange: "transform" }}
                >
                  <svg width="44" height="44" viewBox="0 0 44 44" className="overflow-visible">
                    <g ref={leftEyeShapeRef} style={{ transformOrigin: "22px 22px" }}>
                      <ellipse ref={leftEyeGlowRef} cx="22" cy="22" rx="14" ry="17" fill="#38bdf8" opacity="0" />
                      <rect ref={leftEyePillRef} x="7" y="17.25" width="30" height="9.5" rx="4.75" fill="#ffffff" opacity="0.92" />
                      <ellipse ref={leftEyeGlintRef} cx="17" cy="14" rx="3.2" ry="4.5" fill="#ffffff" opacity="0.12" />
                      <path ref={leftEyeArcRef} d="M 8 22 Q 22 22 36 22" fill="none" stroke="#ffffff" strokeWidth="4.6" strokeLinecap="round" opacity="0" />
                    </g>
                  </svg>
                  <svg width="44" height="44" viewBox="0 0 44 44" className="overflow-visible">
                    <g ref={rightEyeShapeRef} style={{ transformOrigin: "22px 22px" }}>
                      <ellipse ref={rightEyeGlowRef} cx="22" cy="22" rx="14" ry="17" fill="#38bdf8" opacity="0" />
                      <rect ref={rightEyePillRef} x="7" y="17.25" width="30" height="9.5" rx="4.75" fill="#ffffff" opacity="0.92" />
                      <ellipse ref={rightEyeGlintRef} cx="17" cy="14" rx="3.2" ry="4.5" fill="#ffffff" opacity="0.12" />
                      <path ref={rightEyeArcRef} d="M 8 22 Q 22 22 36 22" fill="none" stroke="#ffffff" strokeWidth="4.6" strokeLinecap="round" opacity="0" />
                    </g>
                  </svg>
                </div>
                <svg
                  ref={mouthSvgRef}
                  width="36"
                  height="24"
                  viewBox="0 0 36 24"
                  className="hidden absolute bottom-1.5 left-1/2 -translate-x-1/2 overflow-visible pointer-events-none"
                >
                  <g ref={mouthShapeRef} />
                </svg>

                <div ref={particlesRef} className="hidden absolute inset-0 pointer-events-none">
                  <span ref={(el) => { particleElsRef.current[0] = el; }} className="absolute w-1.5 h-1.5 rounded-full bg-white shadow-[0_0_6px_rgba(255,255,255,0.8)]" />
                  <span ref={(el) => { particleElsRef.current[1] = el; }} className="absolute w-1.5 h-1.5 rounded-full bg-white/85" />
                  <span ref={(el) => { particleElsRef.current[2] = el; }} className="absolute w-2 h-2 rounded-full bg-white/95" />
                  <span ref={(el) => { particleElsRef.current[3] = el; }} className="absolute w-1 h-1 rounded-full bg-white/75" />
                  <span ref={(el) => { particleElsRef.current[4] = el; }} className="absolute w-1.5 h-1.5 rounded-full bg-white/80" />
                </div>

                <div
                  ref={sleepZRef}
                  className="hidden absolute left-1/2 translate-x-10 top-2 text-[11px] font-mono text-white/60 pointer-events-none"
                />
                <div
                  ref={idleDotsRef}
                  className="hidden absolute left-1/2 translate-x-11 top-2.5 text-[11px] font-mono text-white/50 tracking-widest pointer-events-none"
                >
                  ...
                </div>
                <div
                  ref={musicNoteRef}
                  className="hidden absolute left-[calc(50%-76px)] top-1/2 text-[13px] text-sky-300/90 pointer-events-none select-none"
                  style={{ transform: "translateY(-50%)" }}
                >
                  ♫
                </div>
                <div
                  ref={musicEqRef}
                  className="hidden absolute left-[calc(50%+52px)] top-1/2 -translate-y-1/2 flex items-center gap-[3px] h-5 pointer-events-none"
                >
                  {[0, 1, 2, 3].map((idx) => (
                    <span
                      key={idx}
                      ref={(el) => { eqBarRefs.current[idx] = el; }}
                      className="w-[2.5px] rounded-full bg-gradient-to-t from-sky-400 to-white/95"
                      style={{ height: 4 }}
                    />
                  ))}
                </div>

                <div
                  ref={captureToastRef}
                  className="hidden items-center gap-1.5 text-[11px] text-white/90 font-medium ml-2 pr-1 whitespace-nowrap"
                >
                  <span className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full bg-white/15 text-[9px] text-white">
                    ✓
                  </span>
                  <span>{ui.captureToast}</span>
                </div>

                {/* Live Screen Share Left/Right Animations (centered relative to facePivot so never clipped!) */}
                {shareState?.active && !isExpanded && (
                  <>
                    {shareState.analyzing && (
                      <div className="pointer-events-none absolute left-1/2 -translate-x-1/2 top-0 bottom-0 w-[224px] overflow-hidden z-10">
                        <div className="buddy-scan-beam absolute inset-y-0 w-16 bg-gradient-to-r from-transparent via-white/30 to-transparent" />
                      </div>
                    )}

                    {/* Left of Buddy's face: pulsing red LIVE beacon */}
                    <div className="pointer-events-none absolute left-[calc(50%-96px)] top-1/2 -translate-y-1/2 z-20 flex items-center gap-1.5">
                      <span className="relative flex w-2.5 h-2.5 items-center justify-center">
                        {!shareState.paused && !shareState.minimized && (
                          <span className="absolute inset-0 rounded-full bg-red-500/75 animate-ping" />
                        )}
                        <span
                          className={`relative w-2 h-2 rounded-full ${
                            shareState.paused || shareState.minimized
                              ? "bg-white/40"
                              : "bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.95)]"
                          }`}
                        />
                      </span>
                      <span className="text-[8.5px] font-semibold tracking-wider text-white/85">
                        {shareState.paused || shareState.minimized ? "PAUSE" : "LIVE"}
                      </span>
                    </div>

                    {/* Right of Buddy's face: animated optical telemetry waves when idle, quick buttons when hovering */}
                    {buddyState === "peek" ? (
                      <div
                        onPointerDown={(e) => e.stopPropagation()}
                        className="pointer-events-auto absolute left-[calc(50%+54px)] top-1/2 -translate-y-1/2 z-30 flex items-center gap-1"
                      >
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            void invoke("toggle_screen_share_pause").catch(() => {});
                          }}
                          title={shareState.paused ? ui.chat.shareResume : ui.chat.sharePause}
                          className="w-5 h-5 rounded-full bg-white/15 hover:bg-white/25 text-white flex items-center justify-center transition"
                        >
                          {shareState.paused ? (
                            <Play className="w-2.5 h-2.5" />
                          ) : (
                            <Pause className="w-2.5 h-2.5" />
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            void invoke("stop_screen_share").catch(() => {});
                          }}
                          title={ui.chat.shareStop}
                          className="w-5 h-5 rounded-full bg-rose-500/25 hover:bg-rose-500/45 text-rose-200 flex items-center justify-center transition"
                        >
                          <Square className="w-2 h-2 fill-current" />
                        </button>
                      </div>
                    ) : (
                      !musicRef.current.playing && (
                        <div className="pointer-events-none absolute left-[calc(50%+62px)] top-1/2 -translate-y-1/2 z-20 flex items-center gap-[3px] h-4">
                          {[0, 1, 2].map((barIdx) => (
                            <span
                              key={barIdx}
                              className={`w-[2.5px] rounded-full ${
                                shareState.paused || shareState.minimized
                                  ? "bg-white/30 h-1.5"
                                  : shareState.analyzing
                                  ? "buddy-share-bar bg-white h-3.5"
                                  : "buddy-share-bar bg-white/75 h-2.5"
                              }`}
                              style={
                                !shareState.paused && !shareState.minimized
                                  ? {
                                      animationDelay: `${barIdx * 170}ms`,
                                      animationDuration: shareState.analyzing ? "0.65s" : "1.15s",
                                    }
                                  : undefined
                              }
                            />
                          ))}
                        </div>
                      )
                    )}
                  </>
                )}
              </div>

              <button
                type="button"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  const nextAutoCollapse = !settings.auto_collapse_on_blur;
                  setSettings((prev) => ({
                    ...prev,
                    auto_collapse_on_blur: nextAutoCollapse,
                  }));
                  void invoke("set_auto_collapse_on_blur", {
                    enabled: nextAutoCollapse,
                  }).catch(() => {});
                }}
                aria-pressed={!settings.auto_collapse_on_blur}
                title={
                  settings.auto_collapse_on_blur
                    ? ui.pinAutoCollapseTitle
                    : ui.pinKeepOpenTitle
                }
                className={`absolute z-20 w-6 h-6 rounded-full border flex items-center justify-center transition-all duration-150 ${
                  sideDocked
                    ? "top-10 left-1/2 -translate-x-1/2"
                    : dockEdge === "bottom"
                    ? "right-11 bottom-3"
                    : "right-11 top-3.5"
                } ${
                  !settings.auto_collapse_on_blur
                    ? "bg-sky-500/20 border-sky-400/35 text-sky-200 hover:bg-sky-500/30"
                    : "bg-white/[0.06] border-transparent text-white/45 hover:text-white/85 hover:bg-white/15"
                } ${
                  isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"
                }`}
              >
                {!settings.auto_collapse_on_blur ? (
                  <Pin className="w-3 h-3" />
                ) : (
                  <PinOff className="w-3 h-3" />
                )}
              </button>

              <button
                type="button"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  transitionTo(restStateOrMusic());
                }}
                title={ui.collapseTitle}
                className={`absolute z-20 w-6 h-6 rounded-full bg-white/[0.08] hover:bg-white/20 text-white/60 hover:text-white flex items-center justify-center transition-opacity duration-150 ${
                  sideDocked
                    ? "top-3 left-1/2 -translate-x-1/2"
                    : dockEdge === "bottom"
                    ? "right-3.5 bottom-3"
                    : "right-3.5 top-3.5"
                } ${
                  isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"
                }`}
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>

            {/* PRE-MOUNTED EXPANDED WORKBENCH */}
            <div
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
              className={`flex-1 flex flex-col min-w-0 min-h-0 cursor-default transition-opacity duration-150 ${
                sideDocked
                  ? "px-3.5 pt-3 pb-2.5"
                  : dockEdge === "bottom"
                  ? "px-4 pt-3 pb-1.5"
                  : "px-4 pb-2.5"
              } ${
                isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"
              }`}
            >
              {availableUpdate && (
                <div className="mb-2 rounded-xl bg-sky-500/15 border border-sky-400/30 px-2.5 py-1.5 flex items-center justify-between gap-2 shrink-0">
                  <div className="min-w-0 text-[11px] text-sky-100 truncate">
                    <span className="font-medium">{ui.updateBanner}:</span>{" "}
                    <span className="font-mono text-sky-300">v{availableUpdate.version}</span>
                  </div>
                  <button
                    type="button"
                    disabled={updateProgress !== null}
                    onClick={handleInstallUpdate}
                    className="shrink-0 inline-flex items-center gap-1 px-2 py-0.5 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:opacity-60 text-[10.5px] font-medium text-white transition"
                  >
                    <Download className="w-3 h-3" />
                    <span>
                      {updateProgress !== null
                        ? `${ui.updatingPct} ${updateProgress}%`
                        : ui.updateNow}
                    </span>
                  </button>
                </div>
              )}

              {activeTab === "chat" && (
                <ChatTab
                  lang={lang}
                  agentMode={settings.agent_mode}
                  permissionMode={
                    settings.agent_permission_mode === "ask" ||
                    settings.agent_permission_mode === "auto"
                      ? settings.agent_permission_mode
                      : "default"
                  }
                  thinkingLevel={thinkingLevel}
                  thinkingCapability={thinkingCapability}
                  activeStreamId={activeStreamId}
                  messages={messages}
                  conversations={conversations}
                  activeConversationId={activeConversationId}
                  chatInput={chatInput}
                  onChangeInput={setChatInput}
                  onFocusInput={(focused) => {
                    focusedRef.current = focused;
                  }}
                  isThinking={isThinking}
                  attachments={attachments}
                  recentMemory={recentMemory}
                  onDismissRecent={() => setRecentMemory(null)}
                  onRemoveAttachment={(idx) =>
                    setAttachments((prev) => prev.filter((_, i) => i !== idx))
                  }
                  onSend={(p, a) => void handleSendChat(p, a)}
                  onStopStream={handleStopStream}
                  onToggleAgentMode={handleToggleAgentMode}
                  onChangePermissionMode={handleChangePermissionMode}
                  onChangeThinkingLevel={handleChangeThinkingLevel}
                  onResolveApproval={handleResolveApproval}
                  onNewChat={() => void handleNewChat()}
                  onSelectConversation={handleSelectConversation}
                  onDeleteConversation={(id) => void handleDeleteConversation(id)}
                  onOpenSettings={() => switchTab("settings")}
                  enabledPlugins={enabledPluginsMap}
                  installedPlugins={installedPluginsMap}
                  skills={skills}
                  onOpenPlugin={(pluginId, paths) => {
                    if (paths && paths.length > 0) setDroppedPathsHint(paths);
                    setFocusedPluginId(pluginId);
                    switchTab("plugins");
                  }}
                  onQuickOcr={handleQuickOcr}
                  onQuickDownloadUrl={handleQuickDownloadUrl}
                  pluginProgress={pluginProgress}
                  shareState={shareState}
                  modelProfiles={settings.model_profiles ?? []}
                  defaultWatchProfileId={settings.watch_profile_id ?? ""}
                />
              )}

              {(activeTab === "memory" || activeTab === "plugins" || activeTab === "settings") && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center text-white/45">
                      <Loader2 className="w-4 h-4 animate-spin" />
                    </div>
                  }
                >
                  {activeTab === "memory" && (
                    <MemoryTab
                      lang={lang}
                      refreshTrigger={memoryRefresh}
                      enabledPlugins={enabledPluginsMap}
                      onAskAi={(item) => {
                        setAttachments((prev) => [...prev, memoryToAttachment(item)].slice(0, 6));
                        switchTab("chat");
                      }}
                      onOcrExtracted={(ocrText) => {
                        setMessages((prev) => [
                          ...prev,
                          {
                            id: Date.now(),
                            role: "assistant",
                            text: `**Offline OCR:**\n\`\`\`\n${ocrText}\n\`\`\``,
                          },
                        ]);
                        switchTab("chat");
                      }}
                    />
                  )}

                  {activeTab === "plugins" && (
                    <PluginsTab
                      lang={lang}
                      pluginsData={pluginsData}
                      pluginProgress={pluginProgress}
                      lastPickedColor={lastPickedColor}
                      timerState={timerState}
                      droppedPathsHint={droppedPathsHint}
                      focusedPluginId={focusedPluginId}
                      skills={skills}
                      onRefreshPlugins={setPluginsData}
                      onRefreshSkills={setSkills}
                      onCopyText={(text) => {
                        void navigator.clipboard?.writeText(text).catch(() => {});
                      }}
                    />
                  )}

                  {activeTab === "settings" && (
                    <SettingsTab
                      settings={settings}
                      refreshHz={refreshHz}
                      expandedDims={activeDims}
                      appVersion={appVersion}
                      availableUpdate={availableUpdate}
                      updateProgress={updateProgress}
                      onSaved={handleSettingsSaved}
                      onTriggerSnip={() => {
                        transitionTo(restStateOrMusic());
                        window.setTimeout(() => void invoke("trigger_snip"), 180);
                      }}
                      onTestInhale={() => {
                        transitionTo("idle");
                        window.setTimeout(triggerCapture, 220);
                      }}
                      onCheckUpdate={handleCheckUpdate}
                      onInstallUpdate={handleInstallUpdate}
                      onResetDock={handleResetDock}
                    />
                  )}
                </Suspense>
              )}

              {/* ULTRA-MINIMALIST BOTTOM MICRO-DOCK */}
              <div className="mt-2.5 pt-2 border-t border-white/[0.08] flex items-center justify-center gap-6 shrink-0">
                {(
                  [
                    { id: "chat", label: ui.tabs.chat, Icon: MessageSquare },
                    { id: "memory", label: ui.tabs.memory, Icon: Clock },
                    { id: "plugins", label: ui.tabs.plugins, Icon: Puzzle },
                    { id: "settings", label: ui.tabs.settings, Icon: Settings },
                  ] as const
                ).map(({ id, label, Icon }) => {
                  const active = activeTab === id;
                  return (
                    <button
                      key={id}
                      onClick={() => switchTab(id)}
                      className={`flex items-center gap-1.5 text-[11px] font-medium transition ${
                        active ? "text-white" : "text-white/40 hover:text-white/80"
                      }`}
                    >
                      <Icon className="w-3.5 h-3.5" />
                      {active && <span>{label}</span>}
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Floating Whisper / Reply Mini Bubble (kept outside morphLayerRef so text renders crisp at integer pixels) */}
      {floatingBubble &&
        (floatingBubble.kind !== "share_whisper" || shareState?.active) &&
        !isExpanded &&
        buddyState !== "stealth" && (
        <div
          ref={whisperRef}
          onPointerDown={(e) => e.stopPropagation()}
          onMouseEnter={() => setWhisperHovered(true)}
          onMouseLeave={() => setWhisperHovered(false)}
          onClick={() => {
            if (floatingBubble.kind === "reply_notice") {
              void handleOpenBubble();
            }
          }}
          style={
            dockEdge === "bottom"
              ? {
                  left: Math.round((dims.canvas_w - 296) / 2),
                  bottom: Math.round(dims.pad + 50),
                  width: 296,
                }
              : dockEdge === "left"
              ? {
                  left: Math.round(dims.pad + 50),
                  top: Math.max(dims.pad, Math.round((dims.canvas_h - 110) / 2)),
                  width: 280,
                }
              : dockEdge === "right"
              ? {
                  left: Math.round(dims.canvas_w - dims.pad - 50 - 280),
                  top: Math.max(dims.pad, Math.round((dims.canvas_h - 110) / 2)),
                  width: 280,
                }
              : {
                  left: Math.round((dims.canvas_w - 296) / 2),
                  top: Math.round(dims.pad + 50),
                  width: 296,
                }
          }
          className={`pointer-events-auto absolute z-40 rounded-2xl bg-[#0b0d14]/78 backdrop-blur-md shadow-[0_10px_28px_rgba(0,0,0,0.45)] px-3 py-2 text-[12px] text-[#f8fafc] border transition-all duration-200 ease-out origin-center ${
            floatingBubble.kind === "reply_notice" && floatingBubble.tone === "error"
              ? "border-rose-400/35"
              : floatingBubble.kind === "reply_notice" && floatingBubble.tone === "approval"
              ? "border-amber-400/35"
              : "border-white/[0.18]"
          } ${
            bubbleVisible
              ? "opacity-100 translate-x-0 translate-y-0 scale-100"
              : dockEdge === "bottom"
              ? "opacity-0 translate-y-2 scale-95"
              : dockEdge === "left"
              ? "opacity-0 -translate-x-2 scale-95"
              : dockEdge === "right"
              ? "opacity-0 translate-x-2 scale-95"
              : "opacity-0 -translate-y-2 scale-95"
          } ${
            floatingBubble.kind === "reply_notice" ? "cursor-pointer hover:border-white/30" : ""
          }`}
        >
          <div className="min-w-0">
            <p className="leading-snug text-[#f8fafc] break-words">
              {floatingBubble.kind === "share_whisper"
                ? floatingBubble.whisper.text
                : floatingBubble.text}
            </p>
            <div className="mt-1.5 flex items-center justify-end gap-1.5">
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  dismissBubbleAnimated();
                }}
                className="px-2 py-0.5 rounded-full text-[10.5px] text-white/65 hover:text-white hover:bg-white/[0.08] transition"
              >
                {ui.chat.whisperDismiss}
              </button>
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  void handleOpenBubble();
                }}
                className="px-2.5 py-0.5 rounded-full bg-white text-black font-medium text-[10.5px] hover:bg-white/90 transition"
              >
                {floatingBubble.kind === "share_whisper" ? ui.chat.whisperReply : ui.chat.replyView}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
