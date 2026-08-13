// Tauri invoke 封装：把所有后端命令收拢到一处，便于测试 mock 与类型收敛。
import { invoke } from "@tauri-apps/api/core";
import type {
  AppConfig,
  Hotword,
  HotwordImportResult,
  LocalAsrModelEntry,
  LocalModelStatus,
  PolishModelStatus,
  ProviderConfig,
  StylePack,
  SessionSummary,
  SystemInfo,
  TranscribeResult,
  UtteranceRecord,
} from "./types";

export type PermissionKind = "microphone" | "accessibility";
export interface PermissionStatus {
  kind: PermissionKind;
  state: "not_determined" | "granted" | "denied" | "restricted";
  hint: string;
}

// 权限状态 → i18n key（文案由组件用 t() 渲染，随界面语言切换）。
export const permissionLabelKey: Record<PermissionStatus["state"], string> = {
  not_determined: "perm.not_determined",
  granted: "perm.granted",
  denied: "perm.denied",
  restricted: "perm.restricted",
};

export const ipc = {
  ping: () => invoke<string>("ping"),
  getConfig: () => invoke<AppConfig>("get_config"),
  defaultConfig: () => invoke<AppConfig>("default_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_app_config", { config }),
  validateProvider: (provider: ProviderConfig) =>
    invoke<void>("validate_provider", { provider }),
  testCloudConnection: (provider: ProviderConfig) =>
    invoke<string>("test_cloud_connection", { provider }),
  testCloudPolish: () => invoke<string>("test_cloud_polish"),
  listSessions: () => invoke<SessionSummary[]>("list_sessions"),
  listUtterances: (sessionId: string) =>
    invoke<UtteranceRecord[]>("list_utterances", { sessionId }),
  searchUtterances: (query: string) =>
    invoke<UtteranceRecord[]>("search_utterances", { query }),
  deleteSession: (sessionId: string) => invoke<void>("delete_session", { sessionId }),
  checkPermission: (kind: PermissionKind) =>
    invoke<PermissionStatus>("check_permission", { kind }),
  requestAccessibility: () => invoke<boolean>("request_accessibility"),
  requestMicrophone: () => invoke<boolean>("request_microphone"),
  openPermissionSettings: (kind: PermissionKind) =>
    invoke<void>("open_permission_settings", { kind }),
  toggleRecording: () => invoke<boolean>("toggle_recording"),
  getRecordingState: () => invoke<boolean>("get_recording_state"),
  // 音频设备
  listAudioDevices: () => invoke<string[]>("list_audio_devices"),
  testMicrophone: (device?: string | null) =>
    invoke<number>("test_microphone", { device: device ?? null }),
  // 开机自启（macOS Login Items）
  getLaunchAtLogin: () => invoke<boolean>("get_launch_at_login"),
  setLaunchAtLogin: (enabled: boolean) =>
    invoke<void>("set_launch_at_login", { enabled }),
  // 本地模型（sherpa-onnx）
  getSystemInfo: (refresh: boolean = false) =>
    invoke<SystemInfo>("get_system_info", { refresh }),
  listLocalAsrModels: () => invoke<LocalAsrModelEntry[]>("list_local_asr_models"),
  setActiveAsrModel: (modelId: string) =>
    invoke<void>("set_active_asr_model", { modelId }),
  deleteLocalAsrModel: (modelId: string) =>
    invoke<void>("delete_local_asr_model", { modelId }),
  getLocalModelStatus: (mode?: string) =>
    invoke<LocalModelStatus>("get_local_model_status", { mode }),
  installLocalModel: (mode?: string) =>
    invoke<void>("install_local_model", { mode }),
  // 二期：AI 润色
  getPolishModelStatus: () => invoke<PolishModelStatus>("get_polish_model_status"),
  installPolishModel: () => invoke<void>("install_polish_model"),
  // 风格包（F1）
  listStylePacks: () => invoke<StylePack[]>("list_style_packs"),
  setActiveStylePack: (id: string | null) =>
    invoke<void>("set_active_style_pack", { id }),
  upsertStylePack: (pack: StylePack) =>
    invoke<StylePack>("upsert_style_pack", { pack }),
  deleteStylePack: (id: string) => invoke<void>("delete_style_pack", { id }),
  // 热词词典
  listHotwords: () => invoke<Hotword[]>("list_hotwords"),
  addHotword: (word: string, weight: number) =>
    invoke<Hotword>("add_hotword", { word, weight }),
  deleteHotword: (id: string) => invoke<void>("delete_hotword", { id }),
  // D3 文件转录
  transcribeFile: (path: string) =>
    invoke<TranscribeResult>("transcribe_file", { path }),
  importHotwordsCsv: (content: string) =>
    invoke<HotwordImportResult>("import_hotwords_csv", { content }),
  // R6 划词问答
  qaRefreshSelection: () => invoke<string | null>("qa_refresh_selection"),
  qaCancel: () => invoke<void>("qa_cancel"),
  qaInsertLast: () => invoke<string | null>("qa_insert_last"),
  qaClear: () => invoke<void>("qa_clear"),
  qaCopyLast: () => invoke<string | null>("qa_copy_last"),
};
