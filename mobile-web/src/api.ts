export interface PairingConfig {
  v?: number;
  name: string;
  pairId: string;
  token: string;
  lan: string;
  remote: string;
  dir: string;
}

export interface MobileAttachment {
  kind: string;
  label: string;
  text_content?: string | null;
  file_path?: string | null;
}

export interface MobileAgentStep {
  stream_id: string;
  step_id: string;
  tool_name: string;
  summary: string;
  status: string;
  detail?: string | null;
  preview?: string | null;
  is_destructive: boolean;
}

export interface MobileMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  thought?: string;
  attachments?: MobileAttachment[];
  steps?: MobileAgentStep[];
  isError?: boolean;
}

export interface MobileConversation {
  id: number;
  title: string;
  message_count: number;
  source?: "desktop" | "mobile" | string;
  updated_at: number;
}

export interface MobileBrowserState {
  active: boolean;
  watching: boolean;
  user_takeover: boolean;
  navigating: boolean;
  url: string;
  title: string;
  action_label: string;
  needs_attention: boolean;
  last_error: string;
}

export interface MobileMemory {
  id: number;
  kind: string;
  content_text?: string | null;
  file_path?: string | null;
  thumb_path?: string | null;
  app_name: string;
  is_starred: boolean;
  updated_at: number;
}

export interface MobileReminder {
  id: number;
  title: string;
  fire_at_ms: number;
  repeat_rule?: string | null;
  enabled: boolean;
}

export interface MobileBookmark {
  id: number;
  title: string;
  url: string;
}

export interface PingInfo {
  ok: boolean;
  version: string;
  device_name: string;
  lan_url?: string | null;
  remote_url?: string | null;
  thinking: boolean;
  browser: MobileBrowserState;
  model_id: string;
  chat_profile_id: string;
  ui_language: string;
}

export interface AndroidBridge {
  saveMediaFile?: (url: string, filename: string) => void;
  showNotification?: (title: string, body: string, tag: string) => void;
  consumePendingShare?: () => string;
  openNativeQrScanner?: () => void;
  copyText?: (text: string) => boolean;
  getAppVersion?: () => string;
  openUpdateUrl?: (url: string) => void;
}

export interface MobileUpdateInfo {
  versionCode: number;
  versionName: string;
  apkUrl: string;
  notes?: string;
}

declare global {
  interface Window {
    BuddyAndroid?: AndroidBridge;
    __onBuddyQrScanned?: (raw: string) => void;
    __onBuddyAndroidShare?: (jsonPayload: string) => void;
  }
}

const STORAGE_KEY = "buddy_mobile_pairing_v1";

export function loadPairing(): PairingConfig | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PairingConfig>;
    if (!parsed.token) return null;
    return {
      v: parsed.v ?? 1,
      name: parsed.name || "Buddy PC",
      pairId: parsed.pairId || "",
      token: parsed.token,
      lan: parsed.lan || "",
      remote: parsed.remote || "",
      dir: parsed.dir || "https://buddy-companion.marcus-dev00.workers.dev",
    };
  } catch {
    return null;
  }
}

export function savePairing(cfg: PairingConfig | null) {
  if (!cfg) {
    localStorage.removeItem(STORAGE_KEY);
    return;
  }
  localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
}

export function parsePairingInput(raw: string): PairingConfig | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const cleaned = trimmed.replace(/^buddy-pair:\/\//i, "");
    const obj = JSON.parse(cleaned) as Partial<PairingConfig>;
    if (!obj.token) return null;
    return {
      v: obj.v ?? 1,
      name: obj.name || "Buddy PC",
      pairId: obj.pairId || "",
      token: obj.token.trim(),
      lan: (obj.lan || "").trim().replace(/\/$/, ""),
      remote: (obj.remote || "").trim().replace(/\/$/, ""),
      dir: (obj.dir || "https://buddy-companion.marcus-dev00.workers.dev")
        .trim()
        .replace(/\/$/, ""),
    };
  } catch {
    return null;
  }
}

async function sha256Hex(input: string): Promise<string> {
  const enc = new TextEncoder().encode(input);
  const buf = await crypto.subtle.digest("SHA-256", enc);
  return Array.from(new Uint8Array(buf))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function probeEndpoint(
  baseUrl: string,
  token: string,
  timeoutMs: number
): Promise<PingInfo | null> {
  const clean = baseUrl.trim().replace(/\/$/, "");
  if (!clean) return null;
  const ctrl = new AbortController();
  const timer = window.setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const resp = await fetch(`${clean}/api/ping`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: ctrl.signal,
    });
    if (!resp.ok) return null;
    const data = (await resp.json()) as PingInfo;
    return data.ok ? data : null;
  } catch {
    return null;
  } finally {
    window.clearTimeout(timer);
  }
}

export async function resolveAndConnect(cfg: PairingConfig): Promise<{
  baseUrl: string;
  mode: "lan" | "remote";
  ping: PingInfo;
  updatedConfig: PairingConfig;
} | null> {
  // 1. Try fast local LAN first (1.8s)
  if (cfg.lan) {
    const ping = await probeEndpoint(cfg.lan, cfg.token, 1800);
    if (ping) {
      const updated = {
        ...cfg,
        name: ping.device_name || cfg.name,
        remote: ping.remote_url || cfg.remote,
      };
      return { baseUrl: cfg.lan, mode: "lan", ping, updatedConfig: updated };
    }
  }

  // 2. Try saved remote URL (Tailscale / last Cloudflare tunnel)
  if (cfg.remote) {
    const ping = await probeEndpoint(cfg.remote, cfg.token, 5000);
    if (ping) {
      const updated = {
        ...cfg,
        name: ping.device_name || cfg.name,
        lan: ping.lan_url || cfg.lan,
      };
      return { baseUrl: cfg.remote, mode: "remote", ping, updatedConfig: updated };
    }
  }

  // 3. Resolve latest dynamic tunnel URL from Cloudflare Worker Directory
  if (cfg.dir && cfg.pairId && cfg.token) {
    try {
      const authHash = await sha256Hex(`${cfg.pairId}:${cfg.token}`);
      const resp = await fetch(
        `${cfg.dir}/api/directory/resolve?pair_id=${encodeURIComponent(
          cfg.pairId
        )}&auth_hash=${encodeURIComponent(authHash)}`
      );
      if (resp.ok) {
        const data = (await resp.json()) as { remote_url?: string; lan_url?: string };
        const freshRemote = (data.remote_url || "").trim().replace(/\/$/, "");
        if (freshRemote && freshRemote !== cfg.remote) {
          const ping = await probeEndpoint(freshRemote, cfg.token, 6000);
          if (ping) {
            const updated = {
              ...cfg,
              name: ping.device_name || cfg.name,
              remote: freshRemote,
              lan: data.lan_url || cfg.lan,
            };
            return { baseUrl: freshRemote, mode: "remote", ping, updatedConfig: updated };
          }
        }
      }
    } catch {
      // Directory unreachable
    }
  }

  return null;
}

export async function apiFetch<T = Record<string, unknown>>(
  baseUrl: string,
  token: string,
  path: string,
  opts?: { method?: string; body?: unknown }
): Promise<T> {
  const resp = await fetch(`${baseUrl}${path}`, {
    method: opts?.method ?? "GET",
    headers: {
      Authorization: `Bearer ${token}`,
      ...(opts?.body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: opts?.body !== undefined ? JSON.stringify(opts.body) : undefined,
  });
  const data = (await resp.json().catch(() => ({}))) as T & { error?: string };
  if (!resp.ok) {
    throw new Error(data.error || `HTTP ${resp.status}`);
  }
  return data;
}

export function buildFileUrl(
  baseUrl: string,
  token: string,
  filePath: string,
  download = false
): string {
  return `${baseUrl}/api/file?path=${encodeURIComponent(filePath)}&token=${encodeURIComponent(
    token
  )}${download ? "&download=1" : ""}`;
}

export function buildMemoryImageUrl(
  baseUrl: string,
  token: string,
  memoryId: number,
  full = false,
  download = false
): string {
  return `${baseUrl}/api/memory-image?id=${memoryId}&token=${encodeURIComponent(token)}${
    full ? "&full=1" : ""
  }${download ? "&download=1" : ""}`;
}

export async function copyTextReliably(text: string): Promise<boolean> {
  if (!text) return false;
  try {
    if (window.BuddyAndroid?.copyText) {
      if (window.BuddyAndroid.copyText(text)) return true;
    }
  } catch {
    // fallback
  }
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fallback for file:// / WebView
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "true");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    ta.style.left = "-9999px";
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}

export function getLocalAppVersion(): { code: number; name: string; isNativeApk: boolean } {
  try {
    const raw = window.BuddyAndroid?.getAppVersion?.();
    if (raw) {
      const parsed = JSON.parse(raw) as { code?: number; name?: string };
      return {
        code: Number(parsed.code) || 2,
        name: parsed.name || "0.3.0",
        isNativeApk: true,
      };
    }
  } catch {
    // ignore
  }
  return { code: 2, name: "0.3.0", isNativeApk: Boolean(window.BuddyAndroid) };
}

export async function checkMobileAppUpdate(
  dirUrl = "https://buddy-companion.marcus-dev00.workers.dev"
): Promise<MobileUpdateInfo | null> {
  try {
    const clean = dirUrl.trim().replace(/\/$/, "");
    const resp = await fetch(`${clean}/downloads/version.json?t=${Date.now()}`);
    if (!resp.ok) return null;
    const data = (await resp.json()) as Partial<MobileUpdateInfo>;
    if (!data.versionCode || !data.apkUrl) return null;
    return {
      versionCode: Number(data.versionCode),
      versionName: data.versionName || "",
      apkUrl: data.apkUrl,
      notes: data.notes,
    };
  } catch {
    return null;
  }
}
