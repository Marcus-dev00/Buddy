export type BuddyState =
  | "idle"
  | "sleep"
  | "capture"
  | "peek"
  | "music"
  | "expanded"
  | "stealth";

export type TabId = "chat" | "memory" | "plugins" | "settings";
export type MusicStyle = "lofi" | "pop" | "edm" | "vocal";
export type UiLang = "en" | "zh";
export type ThinkingLevel = "off" | "low" | "medium" | "high" | "max";
export type DockEdge = "top" | "bottom" | "left" | "right" | "free";

export interface LayoutDims {
  v_w: number;
  v_h: number;
  h_w: number;
  h_h: number;
  canvas_w: number;
  canvas_h: number;
  pad: number;
}

export interface DockStatePayload {
  dock_edge: DockEdge;
  snap_preview: DockEdge | null;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  body: string;
  date: string | null;
}

export interface PluginInfo {
  id: string;
  kind: "native" | "engine" | string;
  name_en: string;
  name_zh: string;
  desc_en: string;
  desc_zh: string;
  enabled: boolean;
  installed: boolean;
  version: string;
  size_bytes: number;
  sha256: string | null;
  engine_hint: string | null;
}

export interface PluginListResponse {
  plugins: PluginInfo[];
  mirror_prefix: string;
  total_size_bytes: number;
}

export interface PluginProgressPayload {
  plugin_id: string;
  stage: string;
  percent: number;
  detail: string;
}

export interface OcrResult {
  text: string;
  line_count: number;
  language: string;
}

export interface ClipboardParseItem {
  kind: string;
  label: string;
  value: string;
}

export interface ClipboardParseResult {
  raw_length: number;
  items: ClipboardParseItem[];
  cleaned_text: string;
}

export interface LockProcessInfo {
  pid: number;
  app_name: string;
  service_name: string;
  app_type: string;
  restartable: boolean;
}

export interface ColorPickedPayload {
  hex: string;
  rgb: string;
  hsl: string;
  r: number;
  g: number;
  b: number;
}

export interface TimerTickPayload {
  active: boolean;
  remaining_sec: number;
  total_sec: number;
  label: string;
}

export interface PluginToolOutput {
  outputs: string[];
  summary: string;
}

export interface MemoryItem {
  id: number;
  kind: "screenshot" | "clipboard_text" | "clipboard_image" | string;
  content_text: string | null;
  file_path: string | null;
  thumb_path: string | null;
  sha256: string;
  app_name: string;
  is_starred: boolean;
  created_at: number;
  updated_at: number;
}

export interface MusicPayload {
  playing: boolean;
  style: MusicStyle;
  bass: number;
  mid: number;
  high: number;
  beat: boolean;
  title: string;
  artist: string;
}

export interface AttachmentPayload {
  kind: "image" | "text" | "folder" | "file";
  label: string;
  text_content?: string | null;
  file_path?: string | null;
}

export type AgentStepStatus =
  | "running"
  | "waiting_approval"
  | "done"
  | "rejected"
  | "error";

export type ApprovalDecision = "approve_once" | "approve_all_turn" | "reject";

export interface AgentStep {
  stream_id: string;
  step_id: string;
  tool_name: string;
  summary: string;
  status: AgentStepStatus;
  detail?: string | null;
  preview?: string | null;
  is_destructive: boolean;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  thought?: string;
  attachments?: AttachmentPayload[];
  steps?: AgentStep[];
  outputPaths?: string[];
  isError?: boolean;
}

export interface ConversationSummary {
  id: number;
  title: string;
  message_count: number;
  created_at: number;
  updated_at: number;
}

export interface PersonalityOption {
  id: string;
  label: string;
}

export type ThinkingModeOverride = "auto" | "native" | "prompt";

export interface ModelProfile {
  id: string;
  name: string;
  api_provider: "openai" | "anthropic" | "gemini" | string;
  base_url: string;
  model_id: string;
  supports_vision: boolean;
  thinking_mode?: ThinkingModeOverride | string;
  has_api_key: boolean;
}

export interface AppSettings {
  api_provider: "openai" | "anthropic" | "gemini" | string;
  base_url: string;
  model_id: string;
  persona_id: string;
  auto_hide_stealth: boolean;
  music_reactive: boolean;
  launch_at_startup: boolean;
  has_api_key: boolean;
  agent_mode: boolean;
  thinking_level: ThinkingLevel | string;
  agent_workspace: string;
  ui_language: UiLang | string;
  dock_edge?: DockEdge | string;
  dock_cx?: number | null;
  dock_cy?: number | null;
  update_endpoint?: string;
  model_profiles?: ModelProfile[];
  chat_profile_id?: string;
  watch_profile_id?: string;
  auto_collapse_on_blur?: boolean;
}

export interface ShareTargetInfo {
  id: string;
  kind: "auto_cursor" | "monitor" | "window" | string;
  title: string;
  app_name: string;
  width: number;
  height: number;
  is_primary: boolean;
  hwnd: number;
}

export interface ShareStatePayload {
  active: boolean;
  paused: boolean;
  minimized: boolean;
  analyzing: boolean;
  target_id: string;
  target_kind: "monitor" | "window" | string;
  target_label: string;
  target_hwnd: number;
  dock_edge: string;
  goal: string;
  cadence: "quiet" | "balanced" | "active" | string;
  show_border: boolean;
  watch_profile_id: string;
  frame_count: number;
  reason: string;
  last_error?: string;
}

export interface ShareWhisperPayload {
  id: string;
  text: string;
  timestamp_ms: number;
  target_label: string;
}

export interface SkillInfo {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  source: string;
  dir_path: string;
  has_scripts: boolean;
  file_count: number;
  installed_at: number;
}

export interface SkillDetail {
  info: SkillInfo;
  skill_md: string;
  files: string[];
}

export interface GithubSkillCandidate {
  title: string;
  description: string;
  install_spec: string;
  html_url: string;
  stars: number;
  is_repo: boolean;
}
