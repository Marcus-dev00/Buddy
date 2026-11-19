import { useRef } from "react";
import { ShieldAlert, ShieldCheck, Zap } from "lucide-react";
import { getStrings } from "../i18n";
import {
  ALL_THINKING_LEVELS,
  formatThinkingBadge,
  type ModelThinkingCapability,
} from "../thinking";
import type { AgentPermissionMode, ThinkingLevel, UiLang } from "../types";

export const THINKING_STEPS: ThinkingLevel[] = ALL_THINKING_LEVELS;

const DEFAULT_THINKING_CAP: ModelThinkingCapability = {
  levels: ALL_THINKING_LEVELS,
  isNative: false,
  familyName: "Prompt",
};

export function getStepDisplay(
  step: ThinkingLevel,
  lang: UiLang,
  cap: ModelThinkingCapability = DEFAULT_THINKING_CAP
): { short: string; label: string; desc: string } {
  const s = getStrings(lang).chat;
  const base = s.thinkLevels[step];
  const ov = cap.stepOverrides?.[step];
  if (!ov) return base;
  return {
    short: ov.short,
    label: ov.label,
    desc: lang === "zh" ? ov.descZh : ov.descEn,
  };
}

export function ThinkingSlider({
  value,
  lang,
  capability,
  disabled,
  compact,
  onChange,
}: {
  value: ThinkingLevel;
  lang: UiLang;
  capability?: ModelThinkingCapability;
  disabled?: boolean;
  compact?: boolean;
  onChange: (next: ThinkingLevel) => void;
}) {
  const s = getStrings(lang).chat;
  const cap = capability ?? DEFAULT_THINKING_CAP;
  const steps = cap.levels.length > 0 ? cap.levels : THINKING_STEPS;
  const rawIdx = steps.indexOf(value);
  const idx = rawIdx >= 0 ? rawIdx : 0;
  const count = steps.length;
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const pickByClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.max(0, Math.min(0.999, (clientX - rect.left) / rect.width));
    const nextIdx = Math.floor(ratio * count);
    const next = steps[nextIdx];
    if (next && next !== value) {
      onChange(next);
    }
  };

  const activeStep = steps[idx] ?? "medium";
  const accentBar =
    activeStep === "off"
      ? "bg-white/25"
      : activeStep === "low"
      ? "bg-white/60"
      : activeStep === "medium"
      ? "bg-sky-400"
      : activeStep === "high"
      ? "bg-indigo-400"
      : "bg-amber-400 shadow-[0_0_8px_rgba(251,191,36,0.75)]";

  const widthClass = compact
    ? count <= 2
      ? "w-[82px] h-[24px]"
      : count === 3
      ? "w-[110px] h-[24px]"
      : count === 4
      ? "w-[128px] h-[24px]"
      : "w-[144px] h-[24px]"
    : count <= 2
    ? "w-[116px] h-[26px]"
    : count === 3
    ? "w-[148px] h-[26px]"
    : count === 4
    ? "w-[174px] h-[26px]"
    : "w-[196px] h-[26px]";

  const badgeText = formatThinkingBadge(cap, lang);

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label={s.thinkPrefix}
      aria-valuemin={0}
      aria-valuemax={Math.max(0, count - 1)}
      aria-valuenow={idx}
      onPointerDown={(e) => {
        if (disabled) return;
        draggingRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        pickByClientX(e.clientX);
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current || disabled) return;
        pickByClientX(e.clientX);
      }}
      onPointerUp={(e) => {
        if (!draggingRef.current) return;
        draggingRef.current = false;
        try {
          e.currentTarget.releasePointerCapture(e.pointerId);
        } catch {
          // ignore
        }
      }}
      onPointerCancel={() => {
        draggingRef.current = false;
      }}
      style={{ gridTemplateColumns: `repeat(${count}, minmax(0, 1fr))` }}
      className={`relative grid items-center rounded-lg bg-[#08090d] border border-white/[0.09] p-[2px] select-none touch-none ${widthClass} ${
        disabled ? "opacity-45 pointer-events-none" : "cursor-pointer"
      }`}
    >
      <div
        className="pointer-events-none absolute top-[2px] bottom-[2px] left-[2px] rounded-[6px] bg-white/[0.11] border border-white/[0.15] shadow-[0_1px_4px_rgba(0,0,0,0.65)] flex flex-col justify-end items-center pb-[1.5px] transition-transform duration-200 ease-out"
        style={{
          width: `calc((100% - 4px) / ${count})`,
          transform: `translateX(${idx * 100}%)`,
        }}
      >
        <span className={`w-3.5 h-[1.5px] rounded-full transition-colors ${accentBar}`} />
      </div>

      {steps.map((step, i) => {
        const active = i === idx;
        const stepInfo = getStepDisplay(step, lang, cap);
        return (
          <div
            key={step}
            title={`${badgeText} — ${stepInfo.desc}`}
            className={`relative z-10 text-center text-[9.5px] tracking-tight transition-colors ${
              active
                ? step === "max"
                  ? "text-amber-200 font-semibold"
                  : "text-white font-medium"
                : "text-white/40 hover:text-white/75"
            }`}
          >
            {stepInfo.short}
          </div>
        );
      })}
    </div>
  );
}

export const PERMISSION_STEPS: AgentPermissionMode[] = ["ask", "default", "auto"];

export function PermissionSlider({
  value,
  lang,
  onChange,
}: {
  value: AgentPermissionMode;
  lang: UiLang;
  onChange: (next: AgentPermissionMode) => void;
}) {
  const s = getStrings(lang).chat;
  const rawIdx = PERMISSION_STEPS.indexOf(value);
  const idx = rawIdx >= 0 ? rawIdx : 1;
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const pickByClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.max(0, Math.min(0.999, (clientX - rect.left) / rect.width));
    const nextIdx = Math.floor(ratio * PERMISSION_STEPS.length);
    const next = PERMISSION_STEPS[nextIdx];
    if (next && next !== value) {
      onChange(next);
    }
  };

  const accentBar =
    value === "ask"
      ? "bg-cyan-400 shadow-[0_0_8px_rgba(34,211,238,0.65)]"
      : value === "auto"
      ? "bg-rose-500 shadow-[0_0_8px_rgba(244,63,94,0.75)]"
      : "bg-blue-400 shadow-[0_0_8px_rgba(96,165,250,0.65)]";

  return (
    <div
      ref={trackRef}
      role="radiogroup"
      aria-label="Agent permission mode"
      onPointerDown={(e) => {
        draggingRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        pickByClientX(e.clientX);
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current) return;
        pickByClientX(e.clientX);
      }}
      onPointerUp={(e) => {
        if (!draggingRef.current) return;
        draggingRef.current = false;
        try {
          e.currentTarget.releasePointerCapture(e.pointerId);
        } catch {
          // ignore
        }
      }}
      onPointerCancel={() => {
        draggingRef.current = false;
      }}
      style={{ gridTemplateColumns: "repeat(3, minmax(0, 1fr))" }}
      className="relative grid items-center rounded-lg bg-[#08090d] border border-white/[0.09] p-[2px] select-none touch-none w-[168px] h-[24px] cursor-pointer shrink-0"
    >
      <div
        className="pointer-events-none absolute top-[2px] bottom-[2px] left-[2px] rounded-[6px] bg-white/[0.11] border border-white/[0.15] shadow-[0_1px_4px_rgba(0,0,0,0.65)] flex flex-col justify-end items-center pb-[1.5px] transition-transform duration-200 ease-out"
        style={{
          width: "calc((100% - 4px) / 3)",
          transform: `translateX(${idx * 100}%)`,
        }}
      >
        <span className={`w-4 h-[1.5px] rounded-full transition-colors duration-200 ${accentBar}`} />
      </div>

      {(
        [
          { id: "ask", icon: ShieldAlert, activeText: "text-cyan-200 font-medium" },
          { id: "default", icon: ShieldCheck, activeText: "text-blue-200 font-medium" },
          { id: "auto", icon: Zap, activeText: "text-rose-200 font-semibold" },
        ] as const
      ).map((item) => {
        const active = value === item.id;
        const Icon = item.icon;
        const info = s.permModes[item.id];
        return (
          <div
            key={item.id}
            role="radio"
            aria-checked={active}
            title={info.desc}
            className={`relative z-10 flex items-center justify-center gap-0.5 text-[9.5px] tracking-tight transition-colors duration-200 ${
              active ? item.activeText : "text-white/40 hover:text-white/75"
            }`}
          >
            <Icon className="w-2.5 h-2.5 shrink-0" />
            <span>{info.label}</span>
          </div>
        );
      })}
    </div>
  );
}
