import type { RefObject } from "react";
import { Bell } from "lucide-react";
import type { DockEdge, LayoutDims, ShareWhisperPayload } from "../types";

export type FloatingBubble =
  | { kind: "share_whisper"; whisper: ShareWhisperPayload; durationMs: number }
  | {
      kind: "reply_notice";
      text: string;
      tone: "reply" | "error" | "approval";
      durationMs: number;
    }
  | {
      kind: "reminder";
      reminderId: number;
      title: string;
      prefix: string;
      durationMs: number;
    };

interface FloatingBubbleOverlayProps {
  bubble: FloatingBubble;
  bubbleVisible: boolean;
  dockEdge: DockEdge;
  dims: LayoutDims;
  whisperRef: RefObject<HTMLDivElement | null>;
  dismissLabel: string;
  actionLabel: string;
  onHoverChange: (hovered: boolean) => void;
  onDismiss: () => void;
  onOpen: () => void;
}

export function FloatingBubbleOverlay({
  bubble,
  bubbleVisible,
  dockEdge,
  dims,
  whisperRef,
  dismissLabel,
  actionLabel,
  onHoverChange,
  onDismiss,
  onOpen,
}: FloatingBubbleOverlayProps) {
  const posStyle =
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
        };

  return (
    <div
      ref={whisperRef}
      onPointerDown={(e) => e.stopPropagation()}
      onMouseEnter={() => onHoverChange(true)}
      onMouseLeave={() => onHoverChange(false)}
      onClick={() => {
        if (bubble.kind === "reply_notice") {
          onOpen();
        }
      }}
      style={posStyle}
      className={`pointer-events-auto absolute z-40 rounded-2xl bg-[#0b0d14]/82 backdrop-blur-md shadow-[0_10px_28px_rgba(0,0,0,0.45)] px-3 py-2 text-[12px] text-[#f8fafc] border transition-all duration-200 ease-out origin-center ${
        bubble.kind === "reminder"
          ? "border-amber-400/45"
          : bubble.kind === "reply_notice" && bubble.tone === "error"
          ? "border-rose-400/35"
          : bubble.kind === "reply_notice" && bubble.tone === "approval"
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
        bubble.kind === "reply_notice" ? "cursor-pointer hover:border-white/30" : ""
      }`}
    >
      <div className="min-w-0">
        {bubble.kind === "reminder" && (
          <div className="flex items-center gap-1 text-[10px] font-medium text-amber-300 mb-0.5">
            <Bell className="w-3 h-3 shrink-0" />
            <span>{bubble.prefix}</span>
          </div>
        )}
        <p className="leading-snug text-[#f8fafc] break-words">
          {bubble.kind === "share_whisper"
            ? bubble.whisper.text
            : bubble.kind === "reminder"
            ? bubble.title
            : bubble.text}
        </p>
        <div className="mt-1.5 flex items-center justify-end gap-1.5">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDismiss();
            }}
            className="px-2 py-0.5 rounded-full text-[10.5px] text-white/65 hover:text-white hover:bg-white/[0.08] transition"
          >
            {dismissLabel}
          </button>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpen();
            }}
            className="px-2.5 py-0.5 rounded-full bg-white text-black font-medium text-[10.5px] hover:bg-white/90 transition"
          >
            {actionLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
