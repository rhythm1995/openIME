// 与 voice_core 的 serde 形状对齐（前端镜像类型）。

export type ProviderKind = "sherpa" | "bailian";

export interface ProviderConfig {
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  model: string;
  vocabulary_id?: string | null;
}

export type PolishPolicy =
  | "prefer_local"
  | "prefer_cloud"
  | "local_only"
  | "cloud_only"
  | "off";

export interface AppConfig {
  active_provider: number;
  providers: ProviderConfig[];
  hotkey: string;
  mute_other_audio: boolean;
  /** 开机自启（macOS Login Items）；开机自启时应用静默常驻菜单栏 */
  launch_at_login?: boolean;
  /** 本地引擎模式（兼容旧字段）：offline / realtime；以 local_asr_model 为准 */
  local_mode?: string;
  /** 当前启用的本地 ASR 模型 id：zipformer-zh-2025 | sensevoice */
  local_asr_model?: string;
  /** 麦克风设备名（null/未设置 = 系统默认输入） */
  audio_device?: string | null;
  /** 二期：AI 润色总开关 */
  polish_enabled?: boolean;
  polish_policy?: PolishPolicy;
  polish_local_model?: string;
  polish_cloud_model?: string;
  active_persona_id?: string | null;
  polish_timeout_ms?: number;
}

export interface PolishModelStatus {
  installed: boolean;
  downloading: boolean;
  model_id: string;
  file_name: string;
  total_size: number;
  model_path: string;
  llm_feature: boolean;
}

export interface Persona {
  id: string;
  name: string;
  prompt: string;
  is_builtin: boolean;
  ord: number;
  hidden: boolean;
}

export interface SessionSummary {
  id: string;
  title: string;
  started_at: string; // RFC3339
  ended_at: string | null;
  engine: string;
  provider: string;
  model: string;
}

export interface UtteranceRecord {
  id: string;
  session_id: string;
  seq: number;
  final_text: string;
  audio_path: string | null;
  created_at: string;
}

export interface Hotword {
  id: string;
  word: string;
  weight: number;
}

/** 本地引擎安装状态（get_local_model_status）。 */
export interface LocalModelStatus {
  installed: boolean;
  downloading: boolean;
  total_files: number;
  missing_files: string[];
  missing_size: number;
  total_size: number;
  model_root: string;
  model_id?: string;
}

/** 本地 ASR 候选（list_local_asr_models）。 */
export interface LocalAsrModelEntry {
  id: string;
  title: string;
  description: string;
  backend: string;
  recommended: boolean;
  approx_size: number;
  installed: boolean;
  active: boolean;
  missing_size: number;
}

/** 模型下载进度事件（model://download-progress）。 */
export interface ModelDownloadProgress {
  phase: "downloading" | "verifying" | "done" | "error" | string;
  file_index: number;
  file_count: number;
  file_name: string;
  file_downloaded: number;
  file_total: number;
  total_downloaded: number;
  total_size: number;
  speed_bps: number;
  message: string;
  /** ASR 模型 id 或 `polish`，进度条挂在对应卡片 */
  target_id?: string | null;
}
