// 后端错误转译：Rust 侧错误以中文常量存储（不引入 Rust i18n 语言包），
// 前端按「已知报错 → i18n key」映射；未知错误原样显示（运行期网络级
// 错误等不逐条映射，见交付边界）。key 位于 settings.errors.* 与 toast.*，
// en/zh 双写（locales 奇偶守卫保持绿）。
//
// 匹配策略：后端串可能带 thiserror 枚举前缀（如「配置错误: 」），因此
// 顶层模式不锚定行首；嵌套 endpoint 变体锚定 ^…$，避免误匹配外层包装串。
// detail 类字段递归转译（嵌套未知则原样保留在英文句式里）。
import type { TFunction } from "i18next";

/** 组件传入的翻译函数（与 react-i18next 的 t 结构兼容，便于单测用假字典）。 */
export type BackendT = (key: string, values?: Record<string, unknown>) => string;

interface ErrorEntry {
  key: string;
  pattern: RegExp;
  /** 从匹配组提取插值；detail 类字段递归转译。 */
  values?: (m: RegExpMatchArray, t: BackendT) => Record<string, string>;
}

/** validate_hotkeys 的热键名（录音/风格包切换/翻译），英文界面按 hotkeyNames 转译。 */
const HOTKEY_NAMES = new Set(["录音", "风格包切换", "翻译"]);

function hotkeyName(name: string, t: BackendT): string {
  return HOTKEY_NAMES.has(name) ? t(`settings.errors.hotkeyNames.${name}`) : name;
}

const ENTRIES: ErrorEntry[] = [
  // 云端 LLM 必填（AppConfig::check_cloud_llm）。
  {
    key: "settings.errors.cloudLlmEndpointRequired",
    pattern: /云端 LLM 配置不完整：Endpoint 为必填项（需与 API Key 同时填写，或两者都清空）/,
  },
  {
    key: "settings.errors.cloudLlmKeyRequired",
    pattern: /云端 LLM 配置不完整：API Key 为必填项（需与 Endpoint 同时填写，或两者都清空）/,
  },
  // Endpoint 校验（外层包装，detail 递归）。
  {
    key: "settings.errors.cloudLlmEndpointCheckFailed",
    pattern: /云端 LLM Endpoint 校验失败：(.+)$/,
    values: (m, t) => ({ detail: i18nBackendError(m[1], t) }),
  },
  {
    key: "settings.errors.baseUrlCheckFailed",
    pattern: /base_url 校验失败：(.+)$/,
    values: (m, t) => ({ detail: i18nBackendError(m[1], t) }),
  },
  {
    key: "settings.errors.endpointCheckFailed",
    pattern: /endpoint「(.+?)」校验失败：(.+)$/,
    values: (m, t) => ({ url: m[1], detail: i18nBackendError(m[2], t) }),
  },
  { key: "settings.errors.baseUrlScheme", pattern: /base_url 必须以 ws:\/\// },
  // 缺 key / 未配置（polish/cloud.rs、polish/router.rs、translate_router.rs）。
  { key: "settings.errors.cloudPolishMissingKey", pattern: /云端润色缺少 API Key（必填）/ },
  { key: "settings.errors.translateMissingKey", pattern: /翻译需要云端 API Key（必填）/ },
  { key: "settings.errors.qaMissingKey", pattern: /问答需要云端 API Key（必填）/ },
  { key: "settings.errors.cloudPolishNotConfigured", pattern: /云端润色未配置/ },
  { key: "settings.errors.noTranslateBackend", pattern: /无可用翻译后端/ },
  // 热键（commands.rs validate_hotkeys；名称按界面语言转译）。
  {
    key: "settings.errors.hotkeySingleKeyNotAllowed",
    pattern: /仅录音快捷键支持单键 Fn\/CapsLock（(.+?)快捷键请用组合键）/,
    values: (m, t) => ({ name: hotkeyName(m[1], t) }),
  },
  {
    key: "settings.errors.hotkeyUnparsable",
    pattern: /(.{1,8}?)快捷键「(.+?)」无法解析/,
    values: (m, t) => ({ name: hotkeyName(m[1], t), key: m[2] }),
  },
  {
    key: "settings.errors.hotkeyConflict",
    pattern: /快捷键冲突：「(.+?)」与「(.+?)」相同（(.+?)）/,
    values: (m, t) => ({ a: hotkeyName(m[1], t), b: hotkeyName(m[2], t), key: m[3] }),
  },
  // P2 范围（AppConfig::validate_p2_fields）。
  {
    key: "settings.errors.shortPressRange",
    pattern: /短按阈值须在 100\.\.=800 之间，当前 (\d+)（默认 300）/,
    values: (m) => ({ value: m[1] }),
  },
  {
    key: "settings.errors.segDurationRange",
    pattern: /分段时长须在 10\.\.=180 之间，当前 (\d+)（默认 60）/,
    values: (m) => ({ value: m[1] }),
  },
  {
    key: "settings.errors.segOverlapRange",
    pattern: /分段重叠须在 1\.\.=30 之间，当前 (\d+)（默认 4）/,
    values: (m) => ({ value: m[1] }),
  },
  {
    key: "settings.errors.segParamsInvalid",
    pattern: /分段参数非法：须 10≤duration≤180、1≤overlap≤30 且 overlap<duration/,
  },
  // provider validate（ProviderConfig::validate）。
  { key: "settings.errors.sherpaMissingModel", pattern: /sherpa provider 缺少 model/ },
  { key: "settings.errors.bailianMissingBaseUrl", pattern: /bailian provider 缺少 base_url/ },
  { key: "settings.errors.bailianMissingApiKey", pattern: /bailian provider 缺少 api_key/ },
  { key: "settings.errors.bailianMissingModel", pattern: /bailian provider 缺少 model/ },
  {
    key: "settings.errors.cloudAsrMissingBaseUrl",
    pattern: /云端 ASR provider 缺少 base_url（endpoint）/,
  },
  { key: "settings.errors.cloudAsrMissingApiKey", pattern: /云端 ASR provider 缺少 api_key/ },
  { key: "settings.errors.cloudAsrMissingModel", pattern: /云端 ASR provider 缺少 model/ },
  // 保存失败（detail 为 I/O 等运行期错误，原样透传）。
  {
    key: "settings.errors.saveFailed",
    pattern: /保存配置失败：(.+)$/,
    values: (m, t) => ({ detail: i18nBackendError(m[1], t) }),
  },
  // 嵌套 endpoint 变体（EndpointError；锚定避免误匹配外层包装串）。
  {
    key: "settings.errors.endpointInvalidUrl",
    pattern: /^URL 格式无效：(.+)$/,
    values: (m) => ({ detail: m[1] }),
  },
  {
    key: "settings.errors.endpointUnsupportedScheme",
    pattern: /^不支持的 scheme: (.+)$/,
    values: (m) => ({ scheme: m[1] }),
  },
  { key: "settings.errors.endpointMissingHost", pattern: /^URL 缺少 host$/ },
  { key: "settings.errors.endpointBlockedMetadata", pattern: /^禁止的云元数据服务地址（IMDS）$/ },
  { key: "settings.errors.endpointBlockedLinkLocal", pattern: /^禁止的 link-local 地址$/ },
  {
    key: "settings.errors.endpointBlockedCgnat",
    pattern: /^禁止的 CGNAT 地址（100\.64\.0\.0\/10）$/,
  },
  { key: "settings.errors.endpointBlockedReserved", pattern: /^禁止的保留地址$/ },
  {
    key: "settings.errors.endpointPublicRequiresTls",
    pattern: /^公网地址必须使用 https \/ wss$/,
  },
  // toast 高频（lib.rs toast://info）。
  { key: "toast.translateIgnoredRecording", pattern: /录音进行中，翻译键已忽略/ },
  { key: "toast.translateBackendMissing", pattern: /请先配置翻译后端：/ },
];

/** 导出供单测守护「key 在 en/zh 两侧都存在」。 */
export const BACKEND_ERROR_ENTRIES = ENTRIES;

/** 已知后端错误 → 界面语言文案；未知原样返回。 */
export function i18nBackendError(e: unknown, t: BackendT | TFunction): string {
  const raw =
    typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const translate = t as BackendT;
  for (const entry of ENTRIES) {
    const m = raw.match(entry.pattern);
    if (m) return translate(entry.key, entry.values?.(m, translate));
  }
  return raw;
}
