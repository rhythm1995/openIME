// 与 voice_core 的 serde 形状对齐（前端镜像类型）。

export type ProviderKind = "sherpa" | "bailian";

export interface ProviderConfig {
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  model: string;
  vocabulary_id?: string | null;
  language?: string | null;
}

export type PolishPolicy =
  | "prefer_local"
  | "prefer_cloud"
  | "local_only"
  | "cloud_only"
  | "off";

/** 润色程度：off（保持原样）/ light（中度，仅校对）/ heavy（高度，改写润色）。 */
export type PolishMode = "off" | "light" | "heavy";

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
  /** 默认识别语言：zh / en / yue / auto（默认 zh） */
  local_language?: string;
  /** 二期：AI 润色总开关 */
  polish_enabled?: boolean;
  polish_policy?: PolishPolicy;
  polish_local_model?: string;
  polish_cloud_model?: string;
  polish_mode?: PolishMode;
  /** 当前选中的风格包 id（F1，仅 heavy 模式生效；null = 默认 Heavy prompt） */
  active_style_pack_id?: string | null;
  polish_timeout_ms?: number;
}

/** 一个风格包（F1）：用户自定义输出风格的 system prompt。 */
export interface StylePack {
  id: string;
  name: string;
  system_prompt: string;
  is_builtin: boolean;
  ord: number;
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

/** 当前 ASR 模型的热词容量上限与已用数量（get_hotword_limit）。 */
export interface HotwordLimit {
  limit: number;
  current: number;
  model_id: string;
}

/** 热词 CSV 导入结果（import_hotwords_csv）。 */
export interface HotwordImportResult {
  imported: number;
  total: number;
  limit: number;
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
export interface ModelPerfTag {
  tag: string;
  kind: string; // suitable | usable | not_recommended | unknown | light
  reason: string;
  color: string;
}
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
  perf_tag?: ModelPerfTag | null;
}

/** 本机性能（get_system_info，持久化到 settings::system_info）。 */
export interface SystemInfo {
  total_mem: number;
  avail_mem: number;
  cpu_brand: string;
  cpu_cores: number;
  os_version: string;
  disk_free: number;
  is_apple_silicon: boolean;
  collected_at: string;
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
