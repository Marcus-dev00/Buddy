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
}
