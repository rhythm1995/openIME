// 与 voice_core 的 serde 形状对齐（前端镜像类型）。

export type ProviderKind = "sherpa" | "bailian" | "openai_asr" | "multimodal_asr";

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

/** 翻译路由策略：prefer_cloud（有网默认走云）/ prefer_local（本地专翻优先）。 */
export type TranslatePolicy = "prefer_cloud" | "prefer_local";

export type PolishCloudProtocol = "openai_chat" | "anthropic" | "openai_responses";

/** 润色程度：off（保持原样）/ light（中度，仅校对）/ heavy（高度，改写润色）。 */
export type PolishMode = "off" | "light" | "heavy";

/** 快捷键模式（A1）：toggle 切换 / hold 按住说话。 */
export type HotkeyMode = "toggle" | "hold";

/** R7 插入策略：auto（先打字失败粘贴）/ type（只打字）/ paste（只粘贴）。 */
export type InsertStrategy = "auto" | "type" | "paste";

/** R5 角色种类：default（普通指令角色）/ translate（翻译角色）。 */
export type RoleKind = "default" | "translate";

/** R5 输出模式：P1 仅 insert（预留 panel）。 */
export type OutputMode = "insert" | "panel";

export interface AppConfig {
  active_provider: number;
  providers: ProviderConfig[];
  hotkey: string;
  hotkey_mode?: HotkeyMode;
  /** 风格包循环切换快捷键（F1，可选，如 Ctrl+Shift+P） */
  style_switch_hotkey?: string | null;
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
  /** 云端 LLM 协议（openai_chat / anthropic / openai_responses） */
  polish_cloud_protocol?: PolishCloudProtocol;
  /** 云端 LLM endpoint（base URL） */
  polish_cloud_endpoint?: string;
  /** 云端 LLM API Key */
  polish_cloud_api_key?: string;
  polish_mode?: PolishMode;
  /** 当前选中的风格包 id（F1，仅 heavy 模式生效；null = 默认 Heavy prompt） */
  active_style_pack_id?: string | null;
  polish_timeout_ms?: number;
  /** 半角标点偏好 app 关键字（B5） */
  punct_half_width_apps?: string[];
  /** 繁简偏好（B6）：auto / simplified / traditional */
  chinese_script_preference?: string;

  // ── P1：R4 翻译 ──
  /** 翻译快捷键（null = 不注册；P1 仅 Toggle） */
  translate_hotkey?: string | null;
  /** 翻译目标语言（BCP-47 短码，固定下拉） */
  translate_target_lang?: string;
  /** 「先润色再翻译」：云端哨兵合成；本地 = Light 纠错再译（两步） */
  translate_with_polish?: boolean;
  /** 本地专翻模型 id：milmmt-1b | hy-mt-1.8b | ""（未选） */
  translate_local_model?: string;
  /** 弱机兼译：专翻不可用时用润色模型兼做翻译 */
  translate_use_llm_fallback?: boolean;
  /** 翻译路由策略：prefer_cloud（默认）/ prefer_local */
  translate_policy?: TranslatePolicy;

  // ── P1：R5 前缀角色 ──
  /** 识别结果前缀分流到角色（开 → 听写整段插入，关 → 恢复流式上屏） */
  prefix_roles_enabled?: boolean;
  /** 助手名称：「助手名+角色别名」组合触发前缀角色（空 = 关闭） */
  assistant_name?: string;

  // ── P1：R6 划词问答 ──
  /** QA 快捷键（null = 不注册） */
  qa_hotkey?: string | null;
  /** QA 问答写入历史（sessions/utterances） */
  qa_save_history?: boolean;

  // ── P1：R7 粘贴兜底 ──
  insert_strategy?: InsertStrategy;
  /** 前台 app 命中任一条时视同粘贴（应对「Ok 但吞键」） */
  paste_fallback_apps?: string[];
  /** 粘贴后 750ms 恢复原剪贴板 */
  restore_clipboard?: boolean;

  // ── P2：R9 短按补发 ──
  /** Fn 短按阈值（ms）：Hold+Fn 按住超过该时长才开录（默认 300） */
  short_press_ms?: number;
  /** Hold+Fn 短按补发 🌐（默认开） */
  fn_repost_enabled?: boolean;
  /** HID 补发后若前台输入源未变，TIS 切下一输入源（默认关） */
  fn_repost_tis_fallback?: boolean;

  // ── P2：R11 Windows TSF ──
  /** Windows 优先 TSF CommitText 上屏（默认开；非 Windows 忽略） */
  windows_tsf_enabled?: boolean;
  /** TSF 提交失败回退粘贴（默认开） */
  windows_tsf_fallback?: boolean;

  // ── P2：R12 长音频分段 ──
  /** 文件转录切片时长（秒，默认 60） */
  file_seg_duration_secs?: number;
  /** 相邻切片重叠时长（秒，默认 4） */
  file_seg_overlap_secs?: number;
}

/** 一个风格包（F1）：用户自定义输出风格的 system prompt。
 *  R5 扩展：带 match_prefix 时也是「前缀角色」。 */
export interface StylePack {
  id: string;
  name: string;
  system_prompt: string;
  is_builtin: boolean;
  ord: number;
  /** 前缀别名，`|` 分隔（如 邮件|mail|写邮件）；null/空 = 纯风格包 */
  match_prefix?: string | null;
  /** null = cloud（默认）/ cloud / local */
  provider?: string | null;
  /** 覆盖全局 cloud model（可选） */
  model?: string | null;
  /** default / translate */
  role_kind?: RoleKind;
  /** P1 仅 insert */
  output_mode?: OutputMode;
}

/** 本地 LLM 候选（list_local_polish_models / list_local_translate_models）。 */
export interface LlmModelEntry {
  id: string;
  kind: "polish" | "translate";
  title: string;
  description: string;
  approx_size: number;
  installed: boolean;
  active: boolean;
  missing_size: number;
  perf_tag?: ModelPerfTag | null;
  recommended: boolean;
  arch: string;
}

/** 本机三件套概览（get_model_suite_info）。 */
export interface ModelSuiteInfo {
  model_root: string;
  budget_bytes: number;
  used_bytes: number;
  has_cloud: boolean;
  weak_machine: boolean;
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

/** 热词 CSV 导入结果（import_hotwords_csv）。 */
export interface HotwordImportResult {
  imported: number;
  total: number;
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

/** D3 文件转录结果。 */
export interface TranscribeResult {
  text: string;
  srt: string;
  file_name: string;
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
