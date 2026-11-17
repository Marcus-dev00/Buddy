import type {
  AppSettings,
  ModelProfile,
  ThinkingLevel,
  ThinkingModeOverride,
  UiLang,
} from "./types";

export function normalizeThinkingMode(raw?: string): ThinkingModeOverride {
  const s = (raw ?? "").trim().toLowerCase();
  if (s === "native" || s === "prompt") return s;
  return "auto";
}

export interface ThinkingStepCopy {
  short: string;
  label: string;
  descEn: string;
  descZh: string;
}

export interface ModelThinkingCapability {
  levels: ThinkingLevel[];
  isNative: boolean;
  familyName: string;
  stepOverrides?: Partial<Record<ThinkingLevel, ThinkingStepCopy>>;
}

export const ALL_THINKING_LEVELS: ThinkingLevel[] = [
  "off",
  "low",
  "medium",
  "high",
  "max",
];

export function normalizeThinkingLevel(raw: unknown): ThinkingLevel {
  const s = typeof raw === "string" ? raw.trim().toLowerCase() : "";
  if (s === "off" || s === "low" || s === "high" || s === "max") {
    return s;
  }
  return "medium";
}

function classifyProtocol(apiProvider: string, baseUrl: string) {
  const cleanUrl = baseUrl.trim().replace(/\/+$/, "").toLowerCase();
  const isAnthropic = apiProvider === "anthropic" || cleanUrl.includes("anthropic.com");
  const isGemini =
    apiProvider === "gemini" ||
    (cleanUrl.includes("generativelanguage.googleapis.com") && !cleanUrl.includes("/openai"));
  return { isAnthropic, isGemini, cleanUrl };
}

function stripVendorPrefix(modelLower: string): string {
  const trimmed = modelLower.trim().replace(/^models\//, "");
  const slashIdx = trimmed.lastIndexOf("/");
  return slashIdx >= 0 ? trimmed.slice(slashIdx + 1) : trimmed;
}

function parseClaudeVersion(modelLower: string): [number, number] | null {
  const m = stripVendorPrefix(modelLower);
  if (
    !m.includes("claude") &&
    !m.includes("opus") &&
    !m.includes("sonnet") &&
    !m.includes("haiku")
  ) {
    return null;
  }
  const tokens = m.split(/[^a-z0-9]+/).filter(Boolean);
  for (let i = 0; i < tokens.length; i++) {
    const major = Number(tokens[i]);
    if (!Number.isInteger(major) || major < 3 || major > 9) continue;
    const next = i + 1 < tokens.length ? Number(tokens[i + 1]) : NaN;
    const minor = Number.isInteger(next) && next >= 0 && next <= 20 ? next : 0;
    return [major, minor];
  }
  return null;
}

function hasGeminiGen3Plus(modelLower: string): boolean {
  const m = stripVendorPrefix(modelLower);
  return (
    m.includes("gemini-3") ||
    m.includes("gemini3") ||
    m.includes("gemini-4") ||
    m.includes("gemini4")
  );
}

function parseGpt5Minor(modelLower: string): { minor: number; isPro: boolean } | null {
  const m = stripVendorPrefix(modelLower);
  const idx = m.indexOf("gpt-5");
  if (idx < 0) return null;
  const rest = m.slice(idx + "gpt-5".length);
  if (rest.startsWith(".")) {
    const match = rest.slice(1).match(/^(\d+)/);
    return { minor: match ? Number(match[1]) : 0, isPro: m.includes("-pro") };
  }
  if (rest === "" || rest.startsWith("-")) {
    return { minor: 0, isPro: m.includes("-pro") };
  }
  return null;
}

function isOpenAiOSeries(modelLower: string): boolean {
  const m = stripVendorPrefix(modelLower);
  if (m === "o1-mini" || m === "o1-preview") return false;
  return (
    m === "o1" ||
    m.startsWith("o1-") ||
    m === "o3" ||
    m.startsWith("o3-") ||
    m === "o4" ||
    m.startsWith("o4-")
  );
}

const MINIMAL_GEMINI_OVERRIDE: Partial<Record<ThinkingLevel, ThinkingStepCopy>> = {
  off: {
    short: "Min",
    label: "Minimal",
    descEn: "Minimal: fewest native thinking tokens (fastest Gemini 3+)",
    descZh: "Minimal：最少原生思考 token（Gemini 3+ 极速模式）",
  },
};

const GPT5_MINIMAL_OVERRIDE: Partial<Record<ThinkingLevel, ThinkingStepCopy>> = {
  off: {
    short: "Min",
    label: "Minimal",
    descEn: "Minimal: lightest native reasoning effort",
    descZh: "Minimal：最轻原生推理开销",
  },
  max: {
    short: "XHigh",
    label: "XHigh",
    descEn: "XHigh: extended maximum reasoning effort",
    descZh: "XHigh：最高原生推理深度",
  },
};

const MIMO_BINARY_OVERRIDE: Partial<Record<ThinkingLevel, ThinkingStepCopy>> = {
  off: {
    short: "Off",
    label: "Off",
    descEn: "Off: disable native thinking for fast direct reply",
    descZh: "Off：关闭原生思考，直接快速回复",
  },
  high: {
    short: "Think",
    label: "Think",
    descEn: "Think: enable native MiMo reasoning mode",
    descZh: "Think：开启 MiMo 原生深度思考",
  },
};

/**
 * Infers native reasoning levels and UI metadata for the active LLM profile.
 * Must stay aligned with `infer_thinking_capability` in `src-tauri/src/ai_client.rs`.
 */
export function getModelThinkingCapability(
  model?: Pick<ModelProfile, "api_provider" | "base_url" | "model_id" | "thinking_mode"> | null
): ModelThinkingCapability {
  const modeOverride = normalizeThinkingMode(model?.thinking_mode);
  if (modeOverride === "prompt") {
    return {
      levels: ALL_THINKING_LEVELS,
      isNative: false,
      familyName: "Prompt",
    };
  }

  const auto = inferAutoCapability(model);
  if (modeOverride === "native" && !auto.isNative) {
    const apiProvider = model?.api_provider ?? "openai";
    const baseUrl = model?.base_url ?? "";
    const { isAnthropic, isGemini } = classifyProtocol(apiProvider, baseUrl);
    if (isGemini) {
      return {
        levels: ["off", "low", "medium", "high"],
        isNative: true,
        familyName: "Gemini (Native)",
        stepOverrides: MINIMAL_GEMINI_OVERRIDE,
      };
    }
    if (isAnthropic) {
      return {
        levels: ALL_THINKING_LEVELS,
        isNative: true,
        familyName: "Claude (Native)",
      };
    }
    return {
      levels: ["off", "low", "medium", "high"],
      isNative: true,
      familyName: "OpenAI (Native)",
      stepOverrides: GPT5_MINIMAL_OVERRIDE,
    };
  }
  return auto;
}

function inferAutoCapability(
  model?: Pick<ModelProfile, "api_provider" | "base_url" | "model_id"> | null
): ModelThinkingCapability {
  const apiProvider = model?.api_provider ?? "openai";
  const baseUrl = model?.base_url ?? "";
  const modelId = model?.model_id ?? "";
  const { isAnthropic, isGemini, cleanUrl } = classifyProtocol(apiProvider, baseUrl);
  const modelLower = modelId.trim().toLowerCase();
  const modelShort = stripVendorPrefix(modelLower);

  if (isGemini) {
    if (hasGeminiGen3Plus(modelLower)) {
      if (modelShort.includes("pro")) {
        return {
          levels: ["low", "medium", "high"],
          isNative: true,
          familyName: "Gemini 3+ Pro",
        };
      }
      return {
        levels: ["off", "low", "medium", "high"],
        isNative: true,
        familyName: "Gemini 3/3.5",
        stepOverrides: MINIMAL_GEMINI_OVERRIDE,
      };
    }
    if (modelShort.includes("2.5") || modelShort.includes("thinking")) {
      const isPro = modelShort.includes("pro");
      return {
        levels: isPro
          ? ["low", "medium", "high", "max"]
          : ["off", "low", "medium", "high", "max"],
        isNative: true,
        familyName: isPro ? "Gemini 2.5 Pro" : "Gemini 2.5",
      };
    }
    return {
      levels: ALL_THINKING_LEVELS,
      isNative: false,
      familyName: "Gemini (Prompt)",
    };
  }

  if (isAnthropic) {
    const ver = parseClaudeVersion(modelLower);
    if (ver) {
      const [major, minor] = ver;
      if (major > 4 || (major === 4 && minor >= 7)) {
        return {
          levels: ALL_THINKING_LEVELS,
          isNative: true,
          familyName: "Claude Adaptive",
        };
      }
      if (major > 3 || (major === 3 && minor >= 7)) {
        return {
          levels: ALL_THINKING_LEVELS,
          isNative: true,
          familyName: "Claude Thinking",
        };
      }
    }
    return {
      levels: ALL_THINKING_LEVELS,
      isNative: false,
      familyName: "Claude (Prompt)",
    };
  }

  if (cleanUrl.includes("xiaomimimo.com") || modelShort.startsWith("mimo")) {
    return {
      levels: ["off", "high"],
      isNative: true,
      familyName: "MiMo",
      stepOverrides: MIMO_BINARY_OVERRIDE,
    };
  }

  if (cleanUrl.includes("deepseek.com") || modelShort.startsWith("deepseek")) {
    const isReasoner = modelShort.includes("reasoner") || modelShort.includes("r1");
    if (cleanUrl.includes("deepseek.com")) {
      return isReasoner
        ? {
            levels: ["low", "high", "max"],
            isNative: true,
            familyName: "DeepSeek Reasoner",
          }
        : {
            levels: ["off", "low", "high", "max"],
            isNative: true,
            familyName: "DeepSeek",
          };
    }
    return {
      levels: isReasoner ? ["low", "medium", "high", "max"] : ALL_THINKING_LEVELS,
      isNative: false,
      familyName: "DeepSeek (Prompt)",
    };
  }

  if (isOpenAiOSeries(modelLower)) {
    return {
      levels: ["low", "medium", "high"],
      isNative: true,
      familyName: "OpenAI Reasoning",
    };
  }

  const gpt5 = parseGpt5Minor(modelLower);
  if (gpt5) {
    const supportsXhigh = gpt5.minor >= 2 || gpt5.isPro;
    return {
      levels: supportsXhigh
        ? ALL_THINKING_LEVELS
        : ["off", "low", "medium", "high"],
      isNative: true,
      familyName: "GPT-5",
      stepOverrides: GPT5_MINIMAL_OVERRIDE,
    };
  }

  return {
    levels: ALL_THINKING_LEVELS,
    isNative: false,
    familyName: "Prompt",
  };
}

function levelRank(lvl: ThinkingLevel): number {
  switch (lvl) {
    case "off":
      return 0;
    case "low":
      return 1;
    case "medium":
      return 2;
    case "high":
      return 3;
    case "max":
      return 4;
  }
}

export function clampThinkingLevel(
  requested: ThinkingLevel,
  cap: ModelThinkingCapability
): ThinkingLevel {
  if (cap.levels.includes(requested)) {
    return requested;
  }
  const reqRank = levelRank(requested);
  const candidates =
    requested !== "off"
      ? cap.levels.filter((l) => l !== "off")
      : cap.levels;
  const pool = candidates.length > 0 ? candidates : cap.levels;
  let best = pool[0] ?? "medium";
  let bestDist = Infinity;
  for (const cand of pool) {
    const r = levelRank(cand);
    const dist = Math.abs(r - reqRank);
    if (dist < bestDist || (dist === bestDist && r < levelRank(best))) {
      best = cand;
      bestDist = dist;
    }
  }
  return best;
}

export function resolveActiveChatModel(
  settings: AppSettings
): Pick<ModelProfile, "api_provider" | "base_url" | "model_id" | "thinking_mode"> {
  const profiles = settings.model_profiles ?? [];
  const chatId = settings.chat_profile_id?.trim() ?? "";
  const match =
    (chatId ? profiles.find((p) => p.id === chatId) : undefined) ?? profiles[0];
  if (match) {
    return match;
  }
  return {
    api_provider: settings.api_provider,
    base_url: settings.base_url,
    model_id: settings.model_id,
  };
}

export function formatThinkingBadge(
  cap: ModelThinkingCapability,
  lang: UiLang
): string {
  if (lang === "zh") {
    return cap.isNative ? `${cap.familyName} · 原生` : `${cap.familyName} · 提示词思考`;
  }
  return cap.isNative ? `${cap.familyName} · Native` : `${cap.familyName} · Prompt`;
}
