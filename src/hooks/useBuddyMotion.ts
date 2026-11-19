import { useLayoutEffect, useEffect, useRef } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  BuddyState,
  DockEdge,
  LayoutDims,
  MusicStyle,
  ShareStatePayload,
  TabId,
  ThinkingLevel,
} from "../types";

export type IdleBehavior =
  | "long_doze"
  | "settle_drowsy"
  | "startled_wake"
  | "daydream_dots"
  | "eavesdrop"
  | "yawn"
  | "look_around"
  | "wink_play";

export type EyePreset =
  | "open"
  | "doze"
  | "sleep"
  | "happy"
  | "lofi"
  | "pop"
  | "edm"
  | "thinking";

export interface EyePose {
  pillW: number;
  pillH: number;
  pillOp: number;
  glintOp: number;
  arcBend: number; // + = closed sleep U-curve, - = happy/pop ^-curve
  arcOp: number;
  glowOp: number;
  isHappyGreen: boolean;
}

export const EYE_POSES: Record<EyePreset, EyePose> = {
  open: {
    pillW: 26,
    pillH: 34,
    pillOp: 1,
    glintOp: 0.45,
    arcBend: 0,
    arcOp: 0,
    glowOp: 0,
    isHappyGreen: false,
  },
  thinking: {
    pillW: 26,
    pillH: 34,
    pillOp: 1,
    glintOp: 0.48,
    arcBend: 0,
    arcOp: 0,
    glowOp: 0.16,
    isHappyGreen: false,
  },
  lofi: {
    pillW: 29,
    pillH: 15,
    pillOp: 0.96,
    glintOp: 0.34,
    arcBend: 0,
    arcOp: 0,
    glowOp: 0.08,
    isHappyGreen: false,
  },
  doze: {
    pillW: 30,
    pillH: 9.5,
    pillOp: 0.92,
    glintOp: 0.12,
    arcBend: 0,
    arcOp: 0,
    glowOp: 0,
    isHappyGreen: false,
  },
  edm: {
    pillW: 26,
    pillH: 31,
    pillOp: 1,
    glintOp: 0.55,
    arcBend: 0,
    arcOp: 0,
    glowOp: 0.34,
    isHappyGreen: false,
  },
  sleep: {
    pillW: 30,
    pillH: 7,
    pillOp: 0,
    glintOp: 0,
    arcBend: 11,
    arcOp: 0.65,
    glowOp: 0,
    isHappyGreen: false,
  },
  pop: {
    pillW: 28,
    pillH: 8,
    pillOp: 0,
    glintOp: 0,
    arcBend: -15,
    arcOp: 0.96,
    glowOp: 0.1,
    isHappyGreen: false,
  },
  happy: {
    pillW: 28,
    pillH: 8,
    pillOp: 0,
    glintOp: 0,
    arcBend: -15,
    arcOp: 1.0,
    glowOp: 0.14,
    isHappyGreen: false,
  },
};

export interface BuddyMusicRuntime {
  playing: boolean;
  style: MusicStyle;
  bass: number;
  mid: number;
  high: number;
  beatImpulse: number;
  renderedStyle: MusicStyle;
  eqHeights: number[];
  phase: number;
}

export interface TypewriterRuntime {
  streamId: string | null;
  chars: string[];
  shownLen: number;
  streamDone: boolean;
  acc: number;
  lastTick: number;
}

export function stepSpring(
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

export function isSideDock(edge: DockEdge): boolean {
  return edge === "left" || edge === "right";
}

export function headTargetDeg(edge: DockEdge): number {
  return edge === "left" ? 90 : edge === "right" ? -90 : 0;
}

export interface UseBuddyMotionParams {
  setBuddyState: React.Dispatch<React.SetStateAction<BuddyState>>;
  stateRef: React.MutableRefObject<BuddyState>;
  dockEdge: DockEdge;
  dockEdgeRef: React.MutableRefObject<DockEdge>;
  layoutDims: LayoutDims;
  setLayoutDims: React.Dispatch<React.SetStateAction<LayoutDims>>;
  layoutDimsRef: React.MutableRefObject<LayoutDims>;
  thinkingRef: React.MutableRefObject<boolean>;
  thinkingLevelRef: React.MutableRefObject<ThinkingLevel>;
  focusedRef: React.MutableRefObject<boolean>;
  activeTabRef: React.MutableRefObject<TabId>;
  isDraggingRef: React.MutableRefObject<boolean>;
  snapPreviewRef: React.MutableRefObject<DockEdge | null>;
  peekTargetRef: React.MutableRefObject<{ dx: number; dy: number }>;
  shareStateRef: React.MutableRefObject<ShareStatePayload | null>;
  musicRef: React.MutableRefObject<BuddyMusicRuntime>;
  captureTimerRef: React.MutableRefObject<number | null>;
  typewriterRef: React.MutableRefObject<TypewriterRuntime>;
  activeStreamIdRef: React.MutableRefObject<string | null>;
  switchTab: (tab: TabId) => void;
  setRefreshHz: React.Dispatch<React.SetStateAction<number>>;
  onTypewriterFlush: (streamId: string, visibleText: string) => void;
  onTypewriterComplete: () => void;
}

/**
 * Owns Buddy's GPU-composited spring physics loop, parametric SVG eye morphing,
 * idle micro-behaviors, music visualizer overlay, and smooth typewriter drain.
 */
export function useBuddyMotion({
  setBuddyState,
  stateRef,
  dockEdge,
  dockEdgeRef,
  layoutDims,
  setLayoutDims,
  layoutDimsRef,
  thinkingRef,
  thinkingLevelRef,
  focusedRef,
  activeTabRef,
  isDraggingRef,
  snapPreviewRef,
  peekTargetRef,
  shareStateRef,
  musicRef,
  captureTimerRef,
  typewriterRef,
  activeStreamIdRef,
  switchTab,
  setRefreshHz,
  onTypewriterFlush,
  onTypewriterComplete,
}: UseBuddyMotionParams) {
  // Keep latest callbacks accessible inside the long-lived requestAnimationFrame loop.
  const callbacksRef = useRef({
    switchTab,
    onTypewriterFlush,
    onTypewriterComplete,
  });
  callbacksRef.current = { switchTab, onTypewriterFlush, onTypewriterComplete };

  // GPU-composited DOM & SVG refs
  const morphLayerRef = useRef<HTMLDivElement | null>(null);
  const pillShadowRef = useRef<SVGRectElement | null>(null);
  const pillRectRef = useRef<SVGRectElement | null>(null);
  const clipBoxRef = useRef<HTMLDivElement | null>(null);
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
      callbacksRef.current.switchTab("chat");
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

  /** Applies the current spring dimensions and dock edge to the SVG shell, clip-path, and eye stage. */
  const applyPillLayoutGeometry = (dims: LayoutDims, dock: DockEdge) => {
    const s = springRef.current;
    const side = isSideDock(dock);
    const clampedW = Math.min(dims.canvas_w - dims.pad * 2, Math.max(36, s.w));
    const clampedH = Math.min(dims.canvas_h - dims.pad * 2, Math.max(36, s.h));

    let boxX = (dims.canvas_w - clampedW) / 2;
    let boxY = dims.pad;
    if (dock === "left") {
      boxX = dims.pad;
      bySideCenter();
    } else if (dock === "right") {
      boxX = dims.canvas_w - dims.pad - clampedW;
      bySideCenter();
    } else if (dock === "bottom") {
      boxX = (dims.canvas_w - clampedW) / 2;
      boxY = dims.canvas_h - dims.pad - clampedH;
    }

    function bySideCenter() {
      boxY = (dims.canvas_h - clampedH) / 2;
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

    if (eyesStageRef.current) {
      if (side) {
        eyesStageRef.current.style.width = `${s.stageW.toFixed(1)}px`;
        eyesStageRef.current.style.height = "100%";
      } else {
        eyesStageRef.current.style.width = "100%";
        eyesStageRef.current.style.height = `${s.stageH.toFixed(1)}px`;
      }
    }
  };

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
            callbacksRef.current.onTypewriterFlush(tw.streamId, visibleText);
          }
        } else if (tw.streamDone) {
          activeStreamIdRef.current = null;
          tw.streamId = null;
          thinkingRef.current = false;
          callbacksRef.current.onTypewriterComplete();
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
          (isExpandedCtx ? 6 : 4) +
            (m.bass * 0.85 + Math.abs(Math.sin(m.phase)) * 0.15) * (isExpandedCtx ? 20 : 15),
          (isExpandedCtx ? 6 : 4) + (m.mid * 0.8 + m.bass * 0.2) * (isExpandedCtx ? 24 : 17),
          (isExpandedCtx ? 6 : 4) +
            (m.mid * 0.65 + m.high * 0.35 + Math.abs(Math.cos(m.phase * 1.3)) * 0.12) *
              (isExpandedCtx ? 22 : 16),
          (isExpandedCtx ? 6 : 4) +
            (m.high * 0.85 + Math.abs(Math.sin(m.phase * 1.7)) * 0.15) * (isExpandedCtx ? 18 : 13),
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
          const noteLift =
            Math.sin(m.phase * 0.9) * (isExpandedCtx ? 3.2 : 1.8) -
            m.beatImpulse * (isExpandedCtx ? 2.6 : 1.5);
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
            eyeMorphRef.current.target = { ...EYE_POSES.doze, pillH: 8.5 };
            f.eyeSpeed = 7.5;
          } else {
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 5.2;
          }
        } else {
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
            const p = Math.min(1, el / dur);
            const curve =
              p < 0.35
                ? Math.sin((p / 0.35) * (Math.PI / 2))
                : p < 0.65
                ? 1.0 - Math.sin(((p - 0.35) / 0.3) * Math.PI) * 0.08
                : Math.cos(((p - 0.65) / 0.35) * (Math.PI / 2));
            const eased = Math.max(0, curve);
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
        s.tCapRot = 0;
        s.tCapScale = 1;

        if (thinkingRef.current) {
          s.teRot = 0;
          const tl = thinkingLevelRef.current;
          const setEyePulse = (scalePulse: number, opacityPulse: number) => {
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              rightEyeShapeRef.current.style.transform = `scale(${scalePulse.toFixed(3)})`;
              leftEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
              rightEyeShapeRef.current.style.opacity = opacityPulse.toFixed(2);
            }
          };

          if (tl === "off") {
            eyeMorphRef.current.target = { ...EYE_POSES.open };
            f.eyeSpeed = 14.0;
            s.tex = 0;
            s.tey = 0.8;
            idleDotsRef.current?.classList.add("hidden");
            setEyePulse(1, 1);
            if (leftEyeShapeRef.current && rightEyeShapeRef.current) {
              leftEyeShapeRef.current.style.transform = "";
              rightEyeShapeRef.current.style.transform = "";
            }
          } else if (tl === "low") {
            eyeMorphRef.current.target = { ...EYE_POSES.open, glowOp: 0.08 };
            f.eyeSpeed = 10.0;
            s.tex = Math.sin(t * 1.1) * 1.8;
            s.tey = -1.2;
            idleDotsRef.current?.classList.add("hidden");
            const breath = 0.5 + 0.5 * Math.sin(t * 2.8);
            setEyePulse(0.97 + breath * 0.06, 0.82 + breath * 0.18);
          } else if (tl === "high") {
            eyeMorphRef.current.target = { ...EYE_POSES.open, pillW: 27, pillH: 27, glowOp: 0.3 };
            f.eyeSpeed = 9.0;
            s.tex = Math.sin(t * 2.3) * 4.4;
            s.tey = -1.4;
            s.teRot = Math.sin(t * 1.15) * 1.6;
            const breath = 0.5 + 0.5 * Math.sin(t * 5.2);
            idleDotsRef.current?.classList.add("hidden");
            setEyePulse(0.91 + breath * 0.16, 0.58 + breath * 0.42);
          } else if (tl === "max") {
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
            setEyePulse(0.9 + breath * 0.18, 0.54 + breath * 0.46);
          } else {
            eyeMorphRef.current.target = { ...EYE_POSES.thinking };
            f.eyeSpeed = 10.0;
            s.tex = Math.sin(t * 1.6) * 2.8;
            s.tey = -2.0 + Math.cos(t * 2.2) * 1.0;
            idleDotsRef.current?.classList.add("hidden");
            const breath = 0.5 + 0.5 * Math.sin(t * 4.5);
            setEyePulse(0.94 + breath * 0.12, 0.62 + breath * 0.38);
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
      applyPillLayoutGeometry(dims, dock);

      if (morphLayerRef.current) {
        morphLayerRef.current.style.transform =
          st === "expanded"
            ? ""
            : `translateY(${s.capY.toFixed(2)}px) rotate(${s.capRot.toFixed(2)}deg) scale(${s.capScale.toFixed(4)})`;
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
    applyPillLayoutGeometry(layoutDims, dockEdge);
  }, [layoutDims, dockEdge]);

  return {
    refs: {
      morphLayerRef,
      pillShadowRef,
      pillRectRef,
      clipBoxRef,
      eyesStageRef,
      facePivotRef,
      eyesPairRef,
      leftEyeShapeRef,
      rightEyeShapeRef,
      leftEyeGlowRef,
      rightEyeGlowRef,
      leftEyePillRef,
      rightEyePillRef,
      leftEyeGlintRef,
      rightEyeGlintRef,
      leftEyeArcRef,
      rightEyeArcRef,
      mouthSvgRef,
      mouthShapeRef,
      particlesRef,
      particleElsRef,
      sleepZRef,
      idleDotsRef,
      musicNoteRef,
      musicEqRef,
      eqBarRefs,
      captureToastRef,
    },
    springRef,
    renderEyeSvg,
    restStateOrMusic,
    applyVisualTargets,
    transitionTo,
    triggerCapture,
  };
}
