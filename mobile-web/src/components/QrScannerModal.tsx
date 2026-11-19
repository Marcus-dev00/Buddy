import { useEffect, useRef, useState } from "react";
import { Camera, X } from "lucide-react";

interface QrScannerModalProps {
  onScanned: (raw: string) => void;
  onClose: () => void;
}

export function QrScannerModal({ onScanned, onClose }: QrScannerModalProps) {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let stream: MediaStream | null = null;
    let timer: number | null = null;

    const start = async () => {
      try {
        if (!navigator.mediaDevices?.getUserMedia) {
          setErr("当前浏览器不支持调用摄像头，请直接粘贴电脑端复制的配对码。");
          return;
        }
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: "environment" },
          audio: false,
        });
        if (!active) {
          stream.getTracks().forEach((t) => t.stop());
          return;
        }
        if (videoRef.current) {
          videoRef.current.srcObject = stream;
          await videoRef.current.play();
        }

        const BD = (window as unknown as { BarcodeDetector?: new (opts: { formats: string[] }) => { detect: (el: HTMLVideoElement) => Promise<Array<{ rawValue: string }>> } }).BarcodeDetector;
        if (!BD) {
          setErr("当前系统 WebView 未内置扫码解码器，请点击下方「粘贴配对码」完成配对。");
          return;
        }
        const detector = new BD({ formats: ["qr_code"] });
        const tick = async () => {
          if (!active || !videoRef.current) return;
          try {
            const codes = await detector.detect(videoRef.current);
            if (codes.length > 0 && codes[0].rawValue) {
              active = false;
              onScanned(codes[0].rawValue);
              return;
            }
          } catch {
            // keep scanning
          }
          timer = window.setTimeout(() => void tick(), 260);
        };
        void tick();
      } catch (e) {
        setErr(`无法打开摄像头：${String(e)}。可直接使用「粘贴配对码」。`);
      }
    };

    void start();
    return () => {
      active = false;
      if (timer) window.clearTimeout(timer);
      if (stream) stream.getTracks().forEach((t) => t.stop());
    };
  }, [onScanned]);

  return (
    <div className="fixed inset-0 z-50 bg-black/92 backdrop-blur-md flex flex-col items-center justify-center p-5">
      <div className="w-full max-w-sm rounded-3xl bg-[#10131d] border border-white/15 overflow-hidden shadow-2xl">
        <div className="px-4 py-3 flex items-center justify-between border-b border-white/10">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Camera className="w-4 h-4 text-sky-300" />
            <span>扫描电脑 Buddy 二维码</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="w-8 h-8 rounded-full bg-white/10 flex items-center justify-center text-white/75"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="relative aspect-square bg-black">
          <video ref={videoRef} playsInline muted className="w-full h-full object-cover" />
          <div className="absolute inset-10 rounded-2xl border-2 border-sky-400/75 pointer-events-none shadow-[0_0_0_999px_rgba(0,0,0,0.45)]" />
        </div>

        {err && (
          <div className="px-4 py-3 text-xs text-amber-200 bg-amber-500/10 border-t border-amber-400/20 leading-relaxed">
            {err}
          </div>
        )}
      </div>
    </div>
  );
}
