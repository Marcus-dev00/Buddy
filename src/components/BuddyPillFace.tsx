import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import { Maximize2, Pause, Play, Square, X } from "lucide-react";
import type { BuddyState, DockEdge, ShareStatePayload } from "../types";

export interface BuddyPillFaceProps {
  eyesStageRef: React.RefObject<HTMLDivElement | null>;
  facePivotRef: React.RefObject<HTMLDivElement | null>;
  eyesPairRef: React.RefObject<HTMLDivElement | null>;
  leftEyeShapeRef: React.RefObject<SVGGElement | null>;
  rightEyeShapeRef: React.RefObject<SVGGElement | null>;
  leftEyeGlowRef: React.RefObject<SVGEllipseElement | null>;
  rightEyeGlowRef: React.RefObject<SVGEllipseElement | null>;
  leftEyePillRef: React.RefObject<SVGRectElement | null>;
  rightEyePillRef: React.RefObject<SVGRectElement | null>;
  leftEyeGlintRef: React.RefObject<SVGEllipseElement | null>;
  rightEyeGlintRef: React.RefObject<SVGEllipseElement | null>;
  leftEyeArcRef: React.RefObject<SVGPathElement | null>;
  rightEyeArcRef: React.RefObject<SVGPathElement | null>;
  mouthSvgRef: React.RefObject<SVGSVGElement | null>;
  mouthShapeRef: React.RefObject<SVGGElement | null>;
  particlesRef: React.RefObject<HTMLDivElement | null>;
  particleElsRef: React.MutableRefObject<(HTMLSpanElement | null)[]>;
  sleepZRef: React.RefObject<HTMLDivElement | null>;
  idleDotsRef: React.RefObject<HTMLDivElement | null>;
  musicNoteRef: React.RefObject<HTMLDivElement | null>;
  musicEqRef: React.RefObject<HTMLDivElement | null>;
  eqBarRefs: React.MutableRefObject<(HTMLSpanElement | null)[]>;
  captureToastRef: React.RefObject<HTMLDivElement | null>;
  isExpanded: boolean;
  dockEdge: DockEdge;
  sideDocked: boolean;
  buddyState: BuddyState;
  shareState: ShareStatePayload | null;
  isMusicPlaying: boolean;
  captureToastLabel: string;
  shareResumeLabel: string;
  sharePauseLabel: string;
  shareStopLabel: string;
  openWindowModeTitle: string;
  collapseTitle: string;
  onOpenWindowMode: () => void;
  onCollapse: () => void;
}

/**
 * Renders Buddy's hero SVG face stage: parametric morphing eyes, yawn/capture mouth,
 * capture particle swirl, sleep/idle micro-indicators, music EQ, live screen-share beacon,
 * and expanded-mode pin / close buttons.
 */
export function BuddyPillFace({
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
  isExpanded,
  dockEdge,
  sideDocked,
  buddyState,
  shareState,
  isMusicPlaying,
  captureToastLabel,
  shareResumeLabel,
  sharePauseLabel,
  shareStopLabel,
  openWindowModeTitle,
  collapseTitle,
  onOpenWindowMode,
  onCollapse,
}: BuddyPillFaceProps) {
  return (
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
          <span>{captureToastLabel}</span>
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
                  title={shareState.paused ? shareResumeLabel : sharePauseLabel}
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
                  title={shareStopLabel}
                  className="w-5 h-5 rounded-full bg-rose-500/25 hover:bg-rose-500/45 text-rose-200 flex items-center justify-center transition"
                >
                  <Square className="w-2 h-2 fill-current" />
                </button>
              </div>
            ) : (
              !isMusicPlaying && (
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
          onOpenWindowMode();
        }}
        title={openWindowModeTitle}
        className={`absolute z-20 w-6 h-6 rounded-full border border-transparent bg-white/[0.08] hover:bg-white/20 text-white/65 hover:text-white flex items-center justify-center transition-all duration-150 ${
          sideDocked
            ? "top-10 left-1/2 -translate-x-1/2"
            : dockEdge === "bottom"
            ? "right-11 bottom-3"
            : "right-11 top-3.5"
        } ${isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"}`}
      >
        <Maximize2 className="w-3 h-3" />
      </button>

      <button
        type="button"
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onCollapse();
        }}
        title={collapseTitle}
        className={`absolute z-20 w-6 h-6 rounded-full bg-white/[0.08] hover:bg-white/20 text-white/60 hover:text-white flex items-center justify-center transition-opacity duration-150 ${
          sideDocked
            ? "top-3 left-1/2 -translate-x-1/2"
            : dockEdge === "bottom"
            ? "right-3.5 bottom-3"
            : "right-3.5 top-3.5"
        } ${isExpanded ? "opacity-100 pointer-events-auto" : "opacity-0 pointer-events-none"}`}
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}
