import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";

export const PROTOCOL_OPTIONS = [
  { id: "openai", label: "OpenAI Compatible" },
  { id: "anthropic", label: "Anthropic Claude" },
  { id: "gemini", label: "Google Gemini" },
];

export const LANGUAGE_OPTIONS = [
  { id: "en", label: "English" },
  { id: "zh", label: "简体中文" },
];

export interface DarkSelectProps {
  value: string;
  options: { id: string; label: string }[];
  onChange: (id: string) => void;
  onOpen?: () => void;
  compact?: boolean;
}

export function DarkSelect({ value, options, onChange, onOpen, compact }: DarkSelectProps) {
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

export function ToggleSwitch({
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
