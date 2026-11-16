import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function SnipOverlay() {
  const [preview, setPreview] = useState<[string, number, number] | null>(null);
  const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(null);
  const [dragEnd, setDragEnd] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    void invoke<[string, number, number] | null>("get_snip_preview").then((res) => {
      if (res) setPreview(res);
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        void invoke("cancel_snip");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!preview) {
    return <div className="w-screen h-screen bg-black/50" />;
  }

  const [dataUrl, imgW, imgH] = preview;

  const rect =
    dragStart && dragEnd
      ? {
          left: Math.min(dragStart.x, dragEnd.x),
          top: Math.min(dragStart.y, dragEnd.y),
          width: Math.abs(dragEnd.x - dragStart.x),
          height: Math.abs(dragEnd.y - dragStart.y),
        }
      : null;

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button === 2) {
      void invoke("cancel_snip");
      return;
    }
    setDragStart({ x: e.clientX, y: e.clientY });
    setDragEnd({ x: e.clientX, y: e.clientY });
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!dragStart) return;
    setDragEnd({ x: e.clientX, y: e.clientY });
  };

  const handleMouseUp = () => {
    if (!dragStart || !dragEnd || !rect) return;
    setDragStart(null);
    setDragEnd(null);

    if (rect.width < 6 || rect.height < 6) {
      void invoke("cancel_snip");
      return;
    }

    const vw = window.innerWidth || 1;
    const vh = window.innerHeight || 1;
    void invoke("finish_snip", {
      rx: rect.left / vw,
      ry: rect.top / vh,
      rw: rect.width / vw,
      rh: rect.height / vh,
    });
  };

  const pxW = rect ? Math.round((rect.width / (window.innerWidth || 1)) * imgW) : 0;
  const pxH = rect ? Math.round((rect.height / (window.innerHeight || 1)) * imgH) : 0;

  return (
    <div
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onContextMenu={(e) => {
        e.preventDefault();
        void invoke("cancel_snip");
      }}
      className="relative w-screen h-screen overflow-hidden cursor-crosshair select-none"
    >
      <img
        src={dataUrl}
        alt="Screen Capture"
        draggable={false}
        className="w-full h-full object-fill pointer-events-none"
      />
      <div className="absolute inset-0 bg-black/38 pointer-events-none" />

      {rect && rect.width > 2 && rect.height > 2 && (
        <div
          style={{
            left: rect.left,
            top: rect.top,
            width: rect.width,
            height: rect.height,
            boxShadow: "0 0 0 9999px rgba(0, 0, 0, 0.32)",
          }}
          className="absolute border-2 border-sky-400 bg-transparent pointer-events-none"
        >
          <div className="absolute -top-6 left-0 px-2 py-0.5 rounded bg-black/80 text-[11px] font-mono text-white whitespace-nowrap">
            {pxW} × {pxH}
          </div>
        </div>
      )}

      {!rect && (
        <div className="absolute top-4 left-1/2 -translate-x-1/2 px-3 py-1 rounded-full bg-black/75 border border-white/15 text-[11.5px] text-white/85 pointer-events-none">
          Drag to capture · Esc or Right-click to cancel
        </div>
      )}
    </div>
  );
}
