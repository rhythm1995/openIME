// 与 voice_core 的 serde 形状对齐（前端镜像类型）。

export type ProviderKind = "sherpa" | "bailian";

export interface ProviderConfig {
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  model: string;
  vocabulary_id?: string | null;
}

export interface AppConfig {
  active_provider: number;
  providers: ProviderConfig[];
  hotkey: string;
  mute_other_audio: boolean;
  /** 开机自启（macOS Login Items）；开机自启时应用静默常驻菜单栏 */
  launch_at_login?: boolean;
  /** 本地引擎模式：offline（Fn按下录、松开解码）/ realtime（实时流式） */
  local_mode?: string;
  /** 麦克风设备名（null/未设置 = 系统默认输入） */
  audio_device?: string | null;
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
}
