// TDD4：后端错误转译（backendErrors）。
// Rust 侧错误以中文常量存储（不引入 Rust i18n 语言包），前端按
// 「已知报错 → i18n key」映射：英文界面显示英文文案，未知错误原样。
// 用真实 locale 字典构造假 t（不初始化 i18next）：key 缺失返回 key 本身，
// 因此这些测试同时守护「key 在 en/zh 两侧都存在」。
import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import zh from "./locales/zh.json";
import { BACKEND_ERROR_ENTRIES, i18nBackendError, type BackendT } from "./backendErrors";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function lookup(obj: any, path: string): unknown {
  return path
    .split(".")
    .reduce((o, k) => (o == null ? undefined : (o as Record<string, unknown>)[k]), obj);
}

function makeT(dict: unknown): BackendT {
  return (key, values) => {
    const template = lookup(dict, key);
    if (typeof template !== "string") return key;
    if (!values) return template;
    return Object.entries(values).reduce(
      (s, [k, v]) => s.replaceAll(`{{${k}}}`, String(v)),
      template,
    );
  };
}

const tEn = makeT(en);
const tZh = makeT(zh);

describe("i18nBackendError（en 界面）", () => {
  it("云端 LLM 必填两条：带 Error 枚举前缀与裸串两种形态都映射", () => {
    // 用户实际见到的形态（Error::Config 的 thiserror 前缀「配置错误: 」）。
    expect(
      i18nBackendError(
        "配置错误: 云端 LLM 配置不完整：API Key 为必填项（需与 Endpoint 同时填写，或两者都清空）",
        tEn,
      ),
    ).toBe(
      "Incomplete cloud LLM configuration: API key is required (fill it together with the endpoint, or clear both)",
    );
    expect(
      i18nBackendError("云端 LLM 配置不完整：Endpoint 为必填项（需与 API Key 同时填写，或两者都清空）", tEn),
    ).toBe(
      "Incomplete cloud LLM configuration: Endpoint is required (fill it together with the API key, or clear both)",
    );
  });

  it("Endpoint 校验失败：detail 递归转译，未知嵌套原样透传", () => {
    expect(
      i18nBackendError("配置错误: 云端 LLM Endpoint 校验失败：公网地址必须使用 https / wss", tEn),
    ).toBe("Cloud LLM endpoint validation failed: Public addresses must use https / wss");
    expect(i18nBackendError("配置错误: base_url 校验失败：URL 格式无效：empty host", tEn)).toBe(
      "base_url validation failed: Invalid URL: empty host",
    );
    expect(
      i18nBackendError("endpoint「http://x.com」校验失败：禁止的保留地址", tEn),
    ).toBe('Endpoint "http://x.com" failed validation: Blocked reserved address');
    // 未知嵌套（运行期网络级错误不做逐条映射）→ 原样保留在英文句式里。
    expect(i18nBackendError("云端 LLM Endpoint 校验失败：奇怪的错误", tEn)).toBe(
      "Cloud LLM endpoint validation failed: 奇怪的错误",
    );
  });

  it("未配置云端 LLM / 云润色/翻译/问答缺 key 系列", () => {
    expect(i18nBackendError("配置错误: 云端润色缺少 API Key（必填）", tEn)).toBe(
      "Cloud polish is missing an API key (required)",
    );
    expect(i18nBackendError("配置错误: 翻译需要云端 API Key（必填）", tEn)).toBe(
      "Translation requires a cloud API key (required)",
    );
    expect(i18nBackendError("配置错误: 问答需要云端 API Key（必填）", tEn)).toBe(
      "Q&A requires a cloud API key (required)",
    );
    expect(i18nBackendError("云端 LLM 错误: 云端润色未配置", tEn)).toBe(
      "Cloud polish is not configured",
    );
    expect(i18nBackendError("云端 LLM 错误: 无可用翻译后端", tEn)).toBe(
      "No translation backend available",
    );
  });

  it("热键三条：名称按语言转译", () => {
    expect(
      i18nBackendError("仅录音快捷键支持单键 Fn/CapsLock（风格包切换快捷键请用组合键）", tEn),
    ).toBe(
      "Only the recording shortcut supports the single Fn/CapsLock key (the style-pack switch shortcut must be a key combination)",
    );
    expect(i18nBackendError("翻译快捷键「abc」无法解析", tEn)).toBe(
      'translation shortcut "abc" could not be parsed',
    );
    expect(i18nBackendError('快捷键冲突：「录音」与「翻译」相同（fn）', tEn)).toBe(
      'Shortcut conflict: "recording" and "translation" are the same (fn)',
    );
  });

  it("P2 范围校验保留数值", () => {
    expect(i18nBackendError("配置错误: 短按阈值须在 100..=800 之间，当前 999（默认 300）", tEn)).toBe(
      "Short-press threshold must be within 100..=800, got 999 (default 300)",
    );
    expect(i18nBackendError("分段时长须在 10..=180 之间，当前 999（默认 60）", tEn)).toBe(
      "Segment duration must be within 10..=180, got 999 (default 60)",
    );
    expect(i18nBackendError("分段重叠须在 1..=30 之间，当前 99（默认 4）", tEn)).toBe(
      "Segment overlap must be within 1..=30, got 99 (default 4)",
    );
    expect(
      i18nBackendError("分段参数非法：须 10≤duration≤180、1≤overlap≤30 且 overlap<duration", tEn),
    ).toBe("Invalid segmentation parameters: need 10≤duration≤180, 1≤overlap≤30, and overlap<duration");
  });

  it("provider validate 系列", () => {
    expect(i18nBackendError("配置错误: sherpa provider 缺少 model", tEn)).toBe(
      "sherpa provider is missing model",
    );
    expect(i18nBackendError("配置错误: bailian provider 缺少 base_url", tEn)).toBe(
      "bailian provider is missing base_url",
    );
    expect(i18nBackendError("配置错误: bailian provider 缺少 api_key", tEn)).toBe(
      "bailian provider is missing api_key",
    );
    expect(i18nBackendError("配置错误: bailian provider 缺少 model", tEn)).toBe(
      "bailian provider is missing model",
    );
    expect(i18nBackendError("配置错误: 云端 ASR provider 缺少 base_url（endpoint）", tEn)).toBe(
      "Cloud ASR provider is missing base_url (endpoint)",
    );
    expect(i18nBackendError("配置错误: 云端 ASR provider 缺少 api_key", tEn)).toBe(
      "Cloud ASR provider is missing api_key",
    );
    expect(i18nBackendError("配置错误: 云端 ASR provider 缺少 model", tEn)).toBe(
      "Cloud ASR provider is missing model",
    );
  });

  it("保存配置失败：detail 原样透传", () => {
    expect(i18nBackendError("保存配置失败：I/O 错误（permission denied）", tEn)).toBe(
      "Failed to save config: I/O 错误（permission denied）",
    );
  });

  it("toast 高频两条", () => {
    expect(i18nBackendError("录音进行中，翻译键已忽略", tEn)).toBe(
      "Recording in progress; translate shortcut ignored",
    );
    expect(
      i18nBackendError(
        "请先配置翻译后端：云端 LLM（endpoint + API Key），或在设置 → 本地模型下载翻译/润色模型",
        tEn,
      ),
    ).toBe(
      "Set up a translation backend first: cloud LLM (endpoint + API key), or download a translation/polish model under Settings → Local models",
    );
  });

  it("未知错误原样返回（字符串 / Error 对象 / 其它对象）", () => {
    expect(i18nBackendError("云端 LLM 错误: HTTP 502 Bad Gateway", tEn)).toBe(
      "云端 LLM 错误: HTTP 502 Bad Gateway",
    );
    expect(i18nBackendError(new Error("weird failure"), tEn)).toBe("weird failure");
    expect(i18nBackendError({ foo: 1 }, tEn)).toBe("[object Object]");
  });
});

describe("i18nBackendError（zh 界面兜底）", () => {
  it("zh 模板可还原后端原文（静态 + 动态插值）", () => {
    expect(
      i18nBackendError("云端 LLM 配置不完整：API Key 为必填项（需与 Endpoint 同时填写，或两者都清空）", tZh),
    ).toBe("云端 LLM 配置不完整：API Key 为必填项（需与 Endpoint 同时填写，或两者都清空）");
    expect(i18nBackendError("翻译快捷键「ctrl+q」无法解析", tZh)).toBe("翻译快捷键「ctrl+q」无法解析");
    expect(i18nBackendError("快捷键冲突：「录音」与「翻译」相同（fn）", tZh)).toBe(
      "快捷键冲突：「录音」与「翻译」相同（fn）",
    );
    expect(i18nBackendError("短按阈值须在 100..=800 之间，当前 999（默认 300）", tZh)).toBe(
      "短按阈值须在 100..=800 之间，当前 999（默认 300）",
    );
    expect(
      i18nBackendError(
        "请先配置翻译后端：云端 LLM（endpoint + API Key），或在设置 → 本地模型下载翻译/润色模型",
        tZh,
      ),
    ).toBe("请先配置翻译后端：云端 LLM（endpoint + API Key），或在设置 → 本地模型下载翻译/润色模型");
  });
});

describe("映射表与 locale 完整性", () => {
  it("所有条目引用的 key 在 en/zh 两侧都存在（假 t 缺 key 时返回 key 本身）", () => {
    for (const entry of BACKEND_ERROR_ENTRIES) {
      expect(tEn(entry.key), `en 缺 ${entry.key}`).not.toBe(entry.key);
      expect(tZh(entry.key), `zh 缺 ${entry.key}`).not.toBe(entry.key);
    }
    for (const name of ["录音", "风格包切换", "翻译"]) {
      expect(tEn(`settings.errors.hotkeyNames.${name}`), `en 缺 ${name}`).not.toBe(
        `settings.errors.hotkeyNames.${name}`,
      );
      expect(tZh(`settings.errors.hotkeyNames.${name}`), `zh 缺 ${name}`).not.toBe(
        `settings.errors.hotkeyNames.${name}`,
      );
    }
  });
});
