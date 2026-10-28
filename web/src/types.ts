export interface ProjectEntry {
  project_id: string
  name: string
  resolution: string
  stage: string
  created_at: string
  progress: { slides: boolean; script: boolean; person?: boolean; film?: boolean }
}

export interface CmdError {
  ok: false
  error: { code: string; message: string }
}

/** cc-progress 事件（票 13 进度推送）：Tauri listen 或浏览器 mock 发射。 */
export interface ProgressEvent {
  project_id: string
  segment: "tts" | "timeline" | "matte" | "render"
  state: "start" | "progress" | "done" | "skipped" | "failed"
  done?: number | null
  total?: number | null
  message?: string | null
}

/** 四段状态（Step6 展示 + 单段重试）。 */
export interface SegmentState {
  state: "pending" | "running" | "done" | "skipped" | "failed"
  note?: string
  done?: number | null
  total?: number | null
}

export type VoiceEngineKind = "mimo" | "edge"
export type EdgeVoice = "xiaoxiao" | "yunxi"

export interface TtsPreference {
  engine: VoiceEngineKind
  edge_voice: EdgeVoice
}

export type LayoutMode = "free" | "preset"
export type LayoutPreset = "bottom_right" | "bottom_left" | "right_bar"

export interface Layout {
  mode?: LayoutMode
  position?: LayoutPreset
  x_pct?: number
  y_pct?: number
  width_pct: number
  shape: "rounded_rect" | "circle"
  transition: "hard" | "fade"
}

export interface LayoutAnchor {
  x_pct: number
  y_pct: number
  width_pct: number
}
