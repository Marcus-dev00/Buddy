import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clock, Download, MessageSquare, Puzzle, Settings, X } from "lucide-react";
import ChatTab, { memoryToAttachment } from "./components/ChatTab";
import MemoryTab from "./components/MemoryTab";
import SettingsTab from "./components/SettingsTab";
import { getStrings, makeWelcomeGreeting } from "./i18n";
import type {
  AgentStep,
  AppSettings,
  ApprovalDecision,
  AttachmentPayload,
  BuddyState,
  ChatMessage,
  ConversationSummary,
  DockEdge,
  DockStatePayload,
  LayoutDims,
  MemoryItem,
  MusicPayload,
  MusicStyle,
  TabId,
  ThinkingLevel,
  UiLang,
  UpdateInfo,
} from "./types";

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
    arcBend: -16, arcOp: 1.0, glowOp: 0.22, isHappyGreen: true,
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
  const activeConversationIdRef = useRef<number | null>(null);
  const focusedRef = useRef(false);
  const thinkingRef = useRef(false);
  const thinkingLevelRef = useRef<ThinkingLevel>("medium");
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
    const dStr = `M 8 22 Q 22 ${(22 + pose.arcBend).toFixed(2)} 36 22`;

    const applySide = (
      glow: SVGEllipseElement | null,
      pill: SVGRectElement | null,
      glint: SVGEllipseElement | null,
      arc: SVGPathElement | null
    ) => {
      if (glow) {
        glow.setAttribute("opacity", pose.glowOp.toFixed(2));
        glow.setAttribute("fill", pose.isHappyGreen ? "#6ee7b7" : "#38bdf8");
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
        glint.setAttribute("opacity", pose.glintOp.toFixed(2));
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
      s.tEyeSize = 26;
      setCapsuleSizeTarget(152, 42, 21, 42);
      if (f.wasDrowsy) {
        f.idleBehavior = "settle_drowsy";
        f.idleElapsed = 0;
        f.idleDuration = 2.6;
        f.idleEyeMode = "open";
        renderEyeSvg("open", 7.5);
      } else {
        f.idleBehavior = "settle_drowsy";
        f.idleElapsed = 0;
        f.idleDuration = 3.2;
        f.idleEyeMode = "open";
        renderEyeSvg("open", 8.0);
      }
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
      setCapsuleSizeTarget(196, 76, 28, 76, false);
      s.vCapScale = 1.6;
      renderEyeSvg("open", 18.0);
      mouthSvgRef.current?.classList.remove("hidden");
      particlesRef.current?.classList.remove("hidden");
      if (mouthShapeRef.current) {
        mouthShapeRef.current.innerHTML = `<rect x="6" y="4" width="24" height="16" rx="8" fill="#ffffff" />`;
      }
      const side = isSideDock(dockEdgeRef.current);
      void invoke("update_pill_bounds", {
        width: side ? 78 : 250,
        height: side ? 250 : 78,
      });
    } else if (next === "peek") {
      if (musicRef.current.playing) {
        s.tEyeSize = 26;
        setCapsuleSizeTarget(204, 48, 24, 48);
        musicRef.current.renderedStyle = musicRef.current.style;
        if (f.wakeKind === "none") {
          renderEyeSvg("open", 10.0);
        }
      } else {
        s.tEyeSize = 30;
        setCapsuleSizeTarget(186, 50, 25, 50);
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
      s.tCapY = -70;
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
      let loadedLang: UiLang = "en";
      try {
        const cfg = await invoke<AppSettings>("get_settings");
        setSettings(cfg);
        loadedLang = cfg.ui_language === "zh" ? "zh" : "en";
      } catch {
        // ignore
      }
      try {
        const convId = await invoke<number>("ensure_active_conversation");
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
        } else {
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

      if (st === "stealth") {
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
      } else if (st === "sleep") {
        s.tex = 0;
        s.tey = Math.sin(t * 1.2) * 1.4;
        s.tCapScale = 1;
        if (sleepZRef.current) {
          sleepZRef.current.innerHTML = `<span style="opacity:${(0.3 + 0.45 * Math.sin(t * 2.2)).toFixed(2)}">z</span><span style="opacity:${(0.3 + 0.45 * Math.sin(t * 2.2 - 1.1)).toFixed(2)}; font-size:9px; margin-left:1px">Z</span>`;
        }
      } else if (st === "capture") {
        f.captureElapsed += dt;
        if (f.captureElapsed < 0.56) {
          const p = Math.min(1, f.captureElapsed / 0.56);
          const easeIn = p * p;
          const openAmt = Math.sin(p * Math.PI);
          const mw = 14 + openAmt * 15;
          const mh = 6 + openAmt * 16;
          if (mouthShapeRef.current) {
            mouthShapeRef.current.innerHTML = `<rect x="${(18 - mw / 2).toFixed(1)}" y="${(12 - mh / 2).toFixed(1)}" width="${mw.toFixed(1)}" height="${mh.toFixed(1)}" rx="${(mh / 2).toFixed(1)}" fill="#ffffff" />`;
          }
          const inv = 1 - easeIn;
          const swirl = Math.sin(p * Math.PI * 2) * 8 * inv;
          const cx = (isSideDock(dockEdgeRef.current) ? 64 : layoutDimsRef.current.v_w) / 2;
          const offsets: [number, number][] = [
            [cx - 4 - inv * 58 + swirl, 48 + inv * 36],
            [cx + inv * 60 - swirl, 50 + inv * 32],
            [cx - 2 + swirl * 0.7, 49 + inv * 44],
            [cx - 6 - inv * 34, 52 + inv * 40],
            [cx + 2 + inv * 36, 52 + inv * 38],
          ];
          offsets.forEach(([ox, oy], idx) => {
            const el = particleElsRef.current[idx];
            if (el) {
              el.style.left = `${ox.toFixed(1)}px`;
              el.style.top = `${oy.toFixed(1)}px`;
              el.style.opacity = `${(inv * (idx >= 3 ? 0.9 : 1)).toFixed(2)}`;
            }
          });
          s.tex = 0;
          s.tey = -12;
          s.tesx = 1.09;
          s.tesy = 1.09;
          s.tCapRot = Math.sin(p * Math.PI * 2) * 1.6;
        } else if (f.capturePhase === "inhale") {
          f.capturePhase = "swallow";
          particlesRef.current?.classList.add("hidden");
          mouthSvgRef.current?.classList.add("hidden");
          renderEyeSvg("happy", 14.0);
          if (!isSideDock(dockEdgeRef.current)) {
            captureToastRef.current?.classList.remove("hidden");
            captureToastRef.current?.classList.add("flex");
            setCapsuleSizeTarget(248, 44, 22, 44, false);
          } else {
            setCapsuleSizeTarget(196, 44, 22, 44, false);
          }
          s.tey = 0;
          s.vCapScale = -2.6;
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
        morphLayerRef.current.style.transform = `translateY(${s.capY.toFixed(2)}px) rotate(${s.capRot.toFixed(2)}deg) scale(${s.capScale.toFixed(4)})`;
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
  const thinkingLevel: ThinkingLevel =
    settings.thinking_level === "off" ||
    settings.thinking_level === "low" ||
    settings.thinking_level === "high" ||
    settings.thinking_level === "max"
      ? settings.thinking_level
      : "medium";
  thinkingLevelRef.current = thinkingLevel;
  const ui = getStrings(lang);

  return (
    <div className="w-full h-full bg-transparent">
      <div
        ref={morphLayerRef}
        onPointerDown={handleDragPointerDown}
        onPointerMove={handleDragPointerMove}
        onPointerUp={handleDragPointerUp}
        onPointerCancel={handleDragPointerUp}
        className={`relative ${isExpanded ? "cursor-default" : "cursor-grab active:cursor-grabbing"}`}
        style={{
          width: dims.canvas_w,
          height: dims.canvas_h,
          transformOrigin: "center center",
          willChange: "transform",
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
            x={(dims.canvas_w - 152) / 2 - 1.5}
            y={dims.pad + 1}
            width="155"
            height="45"
            rx="22.5"
            fill="rgba(0,0,0,0.34)"
          />
          <rect
            ref={pillRectRef}
            x={(dims.canvas_w - 152) / 2}
            y={dims.pad}
            width="152"
            height="42"
            rx="21"
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
          className="relative w-full h-full"
          style={{
            clipPath: `inset(${dims.pad}px ${(dims.canvas_w - 152) / 2}px ${dims.canvas_h - dims.pad - 42}px ${(dims.canvas_w - 152) / 2}px round 21px)`,
            willChange: "clip-path",
          }}
        >
          <div
            ref={cardFrameRef}
            className="absolute flex"
            style={cardPosStyle}
          >
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
              style={sideDocked ? { width: 42, height: "100%" } : { width: "100%", height: 42 }}
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
                  <span ref={(el) => { particleElsRef.current[0] = el; }} className="absolute w-2.5 h-2.5 rounded-sm bg-white" />
                  <span ref={(el) => { particleElsRef.current[1] = el; }} className="absolute w-2 h-2 rounded-full bg-sky-300" />
                  <span ref={(el) => { particleElsRef.current[2] = el; }} className="absolute w-3 h-2 rounded-sm bg-emerald-300" />
                  <span ref={(el) => { particleElsRef.current[3] = el; }} className="absolute w-1.5 h-1.5 rounded-full bg-white/90" />
                  <span ref={(el) => { particleElsRef.current[4] = el; }} className="absolute w-2 h-2 rounded-sm bg-sky-200" />
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
                  className="hidden items-center gap-1.5 text-[11px] text-emerald-300 font-medium ml-2 pr-1"
                >
                  <span>{ui.captureToast}</span>
                </div>
              </div>

              <button
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
                  thinkingLevel={thinkingLevel}
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
                  onChangeThinkingLevel={handleChangeThinkingLevel}
                  onResolveApproval={handleResolveApproval}
                  onNewChat={() => void handleNewChat()}
                  onSelectConversation={handleSelectConversation}
                  onDeleteConversation={(id) => void handleDeleteConversation(id)}
                  onOpenSettings={() => switchTab("settings")}
                />
              )}

              {activeTab === "memory" && (
                <MemoryTab
                  lang={lang}
                  refreshTrigger={memoryRefresh}
                  onAskAi={(item) => {
                    setAttachments((prev) => [...prev, memoryToAttachment(item)].slice(0, 6));
                    switchTab("chat");
                  }}
                />
              )}

              {activeTab === "plugins" && (
                <div className="flex-1 flex flex-col items-center justify-center text-center px-4 space-y-2">
                  <Puzzle className="w-7 h-7 text-emerald-400/70" />
                  <div className="text-[13px] font-medium text-white">{ui.plugins.title}</div>
                  <p className="text-[11.5px] text-white/50">{ui.plugins.desc}</p>
                </div>
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
    </div>
  );
}
