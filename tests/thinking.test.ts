import test from "node:test";
import assert from "node:assert/strict";
import {
  clampThinkingLevel,
  formatThinkingBadge,
  getModelThinkingCapability,
  normalizeThinkingLevel,
  normalizeThinkingMode,
  resolveActiveChatModel,
} from "../src/thinking.ts";

test("normalizeThinkingMode & normalizeThinkingLevel fall back cleanly", () => {
  assert.equal(normalizeThinkingMode("NATIVE"), "native");
  assert.equal(normalizeThinkingMode("Prompt"), "prompt");
  assert.equal(normalizeThinkingMode("unknown"), "auto");

  assert.equal(normalizeThinkingLevel("HIGH"), "high");
  assert.equal(normalizeThinkingLevel("bogus"), "medium");
  assert.equal(normalizeThinkingLevel(null), "medium");
});

test("getModelThinkingCapability matches model families and manual overrides", () => {
  const g35 = getModelThinkingCapability({
    api_provider: "gemini",
    base_url: "https://generativelanguage.googleapis.com",
    model_id: "gemini-3.5-flash-lite",
  });
  assert.equal(g35.isNative, true);
  assert.deepEqual(g35.levels, ["off", "low", "medium", "high"]);
  assert.equal(clampThinkingLevel("max", g35), "high");

  const mimo = getModelThinkingCapability({
    api_provider: "openai",
    base_url: "https://token-plan-sgp.xiaomimimo.com/v1",
    model_id: "mimo-v2.6-flash",
  });
  assert.equal(mimo.isNative, true);
  assert.deepEqual(mimo.levels, ["off", "high"]);
  assert.equal(clampThinkingLevel("low", mimo), "high");
  assert.equal(clampThinkingLevel("off", mimo), "off");

  const forcedPrompt = getModelThinkingCapability({
    api_provider: "gemini",
    base_url: "https://generativelanguage.googleapis.com",
    model_id: "gemini-3.5-flash-lite",
    thinking_mode: "prompt",
  });
  assert.equal(forcedPrompt.isNative, false);
  assert.equal(forcedPrompt.levels.length, 5);
});

test("resolveActiveChatModel & formatThinkingBadge work consistently", () => {
  const resolved = resolveActiveChatModel({
    api_provider: "openai",
    base_url: "https://api.openai.com/v1",
    model_id: "gpt-4o-mini",
    persona_id: "warm_companion",
    auto_hide_stealth: false,
    music_reactive: true,
    launch_at_startup: false,
    has_api_key: false,
    agent_mode: true,
    thinking_level: "medium",
    agent_workspace: "",
    ui_language: "zh",
    chat_profile_id: "p2",
    model_profiles: [
      {
        id: "p1",
        name: "Default",
        api_provider: "openai",
        base_url: "https://api.openai.com/v1",
        model_id: "gpt-4o-mini",
        supports_vision: true,
        has_api_key: false,
      },
      {
        id: "p2",
        name: "MiMo",
        api_provider: "openai",
        base_url: "https://api.xiaomimimo.com/v1",
        model_id: "mimo-v2-flash",
        supports_vision: true,
        has_api_key: true,
      },
    ],
  });
  assert.equal(resolved.model_id, "mimo-v2-flash");

  const badgeZh = formatThinkingBadge(
    { levels: ["off", "high"], isNative: true, familyName: "MiMo" },
    "zh"
  );
  assert.equal(badgeZh, "MiMo · 原生");

  const claude = getModelThinkingCapability({
    api_provider: "anthropic",
    base_url: "https://api.anthropic.com",
    model_id: "claude-sonnet-4-5",
  });
  assert.equal(claude.isNative, true);
  assert.ok(claude.levels.includes("high"));

  const o3 = getModelThinkingCapability({
    api_provider: "openai",
    base_url: "https://api.openai.com/v1",
    model_id: "o3-mini",
  });
  assert.equal(o3.isNative, true);
  assert.equal(clampThinkingLevel("off", o3), o3.levels[0]);
});
