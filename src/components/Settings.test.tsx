import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import Settings from "./Settings";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

// 与 Rust AppConfig::default + p1-design「AppConfig 默认值」表对齐。
const defaultConfig = {
  active_provider: 0,
  providers: [
    { kind: "sherpa", base_url: "", api_key: "", model: "sensevoice" },
  ],
  hotkey: "Fn",
  hotkey_mode: "hold",
  mute_other_audio: false,
  polish_enabled: false,
  polish_mode: "off",
  polish_policy: "prefer_local",
  polish_cloud_model: "qwen-turbo",
  polish_cloud_protocol: "openai_chat",
  polish_cloud_endpoint: "",
  polish_cloud_api_key: "",
  local_asr_model: "sensevoice",
  // P1 新字段（PR2/PR4/PR5/PR6 默认值）。
  translate_hotkey: null,
  translate_target_lang: "en",
  translate_with_polish: false,
  prefix_roles_enabled: true,
  qa_hotkey: null,
  qa_save_history: false,
  insert_strategy: "auto",
  paste_fallback_apps: [],
  restore_clipboard: true,
  // P2 新字段（PR0 默认值）。
  short_press_ms: 300,
  fn_repost_enabled: true,
  fn_repost_tis_fallback: false,
  windows_tsf_enabled: true,
  windows_tsf_fallback: true,
  file_seg_duration_secs: 60,
  file_seg_overlap_secs: 4,
};

function mockInvoke(map: Record<string, unknown>) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd in map) {
      const v = map[cmd];
      return v instanceof Error ? Promise.reject(v) : Promise.resolve(v);
    }
    return Promise.resolve(undefined);
  });
}

/** 引擎类型 select：含 sherpa 选项的那个（页面新增了插入策略等 select）。 */
function engineSelect(container: HTMLElement): HTMLSelectElement {
  const selects = Array.from(container.querySelectorAll("select"));
  const found = selects.find((s) =>
    Array.from(s.options).some((o) => o.value === "sherpa"),
  );
  return found as HTMLSelectElement;
}

describe("Settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("渲染 4 种识别引擎选项", async () => {
    mockInvoke({ get_config: defaultConfig });
    const { container } = render(<Settings />);
    await screen.findByText("识别引擎");
    const select = engineSelect(container);
    expect(select.value).toBe("sherpa");
    for (const label of [
      "sherpa-onnx（本地模型，隐私，推荐）",
      "百炼 WebSocket 流式（云端）",
      "OpenAI 兼容 REST（OpenRouter/OpenAI 等）",
      "Multimodal REST（百炼 Qwen3 ASR 非流式）",
    ]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
  });

  it("切到 OpenAI 兼容 REST 显示 endpoint 输入", async () => {
    mockInvoke({ get_config: defaultConfig });
    const { container } = render(<Settings />);
    await screen.findByText("识别引擎");
    fireEvent.change(engineSelect(container), { target: { value: "openai_asr" } });
    expect(await screen.findByText("Endpoint（HTTP 地址）")).toBeTruthy();
  });

  it("点击测试连接调用 test_cloud_connection", async () => {
    mockInvoke({ get_config: defaultConfig });
    const { container } = render(<Settings />);
    await screen.findByText("识别引擎");
    fireEvent.change(engineSelect(container), { target: { value: "openai_asr" } });
    const btn = await screen.findByRole("button", { name: /测试连接/ });
    fireEvent.click(btn);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "test_cloud_connection",
        expect.objectContaining({
          provider: expect.objectContaining({ kind: "openai_asr" }),
        }),
      ),
    );
  });

  it("渲染润色三档卡片", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings view="ai" />);
    for (const label of ["保持原样", "中度润色", "高度润色"]) {
      expect(await screen.findByText(label)).toBeTruthy();
    }
  });

  it("润色开着时显示 LLM 协议选择", async () => {
    mockInvoke({
      get_config: { ...defaultConfig, polish_mode: "light" },
    });
    render(<Settings view="ai" />);
    expect(await screen.findByText("云端 LLM 协议")).toBeTruthy();
    expect(screen.getByText("OpenAI Chat Completions（/chat/completions）")).toBeTruthy();
    expect(screen.getByText("Anthropic Messages（/v1/messages）")).toBeTruthy();
    expect(screen.getByText("OpenAI Responses（/v1/responses）")).toBeTruthy();
  });

  // ── P1 新字段 ──

  it("渲染翻译设置（目标语言下拉 + 先润色再翻译）", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings view="ai" />);
    expect(await screen.findByText("翻译")).toBeTruthy();
    expect(screen.getByText("目标语言")).toBeTruthy();
    expect(screen.getByText("先润色再翻译")).toBeTruthy();
    // 目标语言下拉默认 en。
    const langSelect = screen
      .getByText("目标语言")
      .closest(".field")!
      .querySelector("select") as HTMLSelectElement;
    expect(langSelect.value).toBe("en");
    for (const code of ["zh", "ja", "ko", "fr", "de", "es"]) {
      expect(
        Array.from(langSelect.options).some((o) => o.value === code),
      ).toBeTruthy();
    }
  });

  it("渲染插入策略与剪贴板恢复开关", async () => {
    mockInvoke({ get_config: defaultConfig });
    const { container } = render(<Settings />);
    expect(await screen.findByText("插入与剪贴板")).toBeTruthy();
    const strategy = Array.from(container.querySelectorAll("select")).find((s) =>
      Array.from(s.options).some((o) => o.value === "paste"),
    ) as HTMLSelectElement;
    expect(strategy.value).toBe("auto");
    expect(screen.getByText("恢复原剪贴板")).toBeTruthy();
  });

  it("渲染前缀角色卡片与开关", async () => {
    mockInvoke({ get_config: defaultConfig, list_style_packs: [] });
    render(<Settings view="ai" />);
    expect(await screen.findByText("角色 / 风格包")).toBeTruthy();
    expect(screen.getByText("启用前缀角色")).toBeTruthy();
  });

  it("渲染划词问答设置", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings view="ai" />);
    expect(await screen.findByText("划词问答")).toBeTruthy();
    expect(screen.getByText("问答写入历史")).toBeTruthy();
  });

  it("快捷键卡片包含翻译 / QA 输入", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    expect(await screen.findByText("翻译快捷键（可选）")).toBeTruthy();
    expect(screen.getByText("划词问答快捷键（可选）")).toBeTruthy();
  });

  it("触发模式默认按住说话且为第一项；语音视图不含 AI 卡", async () => {
    mockInvoke({ get_config: { ...defaultConfig, hotkey_mode: undefined } });
    render(<Settings />);
    await screen.findByText("快捷键");
    const modeSelect = screen
      .getByText("触发模式")
      .closest(".field")!
      .querySelector("select") as HTMLSelectElement;
    expect(modeSelect.value).toBe("hold");
    expect(modeSelect.options[0].value).toBe("hold");
    // 分组拆分后：语音视图不渲染 AI 增强卡片。
    expect(screen.queryByText("AI 润色")).toBeNull();
    expect(screen.queryByText("角色 / 风格包")).toBeNull();
  });

  it("角色 master-detail：列表选中 + 提示词失焦保存 + AI 视图含润色卡", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_style_packs: [
        { id: "b-mail", name: "邮件助手", system_prompt: "写一封正式邮件", is_builtin: true, ord: 0, match_prefix: "邮件|mail", provider: null, model: null, role_kind: "mail", output_mode: "insert" },
        { id: "u-1", name: "我的风格", system_prompt: "简洁", is_builtin: false, ord: 100, match_prefix: null, provider: null, model: null, role_kind: "default", output_mode: "insert" },
      ],
    });
    render(<Settings view="ai" />);
    expect(await screen.findByText("AI 润色")).toBeTruthy();
    // 内置包名字带「内置」后缀，用正则匹配列表项。
    expect(await screen.findByText(/邮件助手/)).toBeTruthy();
    // 默认选中第一项（邮件助手）→ 编辑面板回填其前缀。
    expect(await screen.findByDisplayValue("邮件|mail")).toBeTruthy();
    // 切到自定义包，改提示词后失焦 → upsert_style_pack。
    fireEvent.click(screen.getByText("我的风格"));
    const prompt = await screen.findByDisplayValue("简洁");
    fireEvent.blur(prompt, { target: { value: "更简洁" } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "upsert_style_pack",
        expect.objectContaining({
          pack: expect.objectContaining({ id: "u-1", system_prompt: "更简洁" }),
        }),
      ),
    );
  });

  it("修改配置后防抖自动保存（无保存按钮），带上 P1 新字段", async () => {
    mockInvoke({ get_config: defaultConfig, list_style_packs: [] });
    render(<Settings />);
    await screen.findByText("识别引擎");
    // 保存按钮已移除：全部即改即存。
    expect(screen.queryByRole("button", { name: /保存设置/ })).toBeNull();
    // 改触发模式 → 500ms 防抖后自动 save_app_config。
    const modeSelect = screen
      .getByText("触发模式")
      .closest(".field")!
      .querySelector("select") as HTMLSelectElement;
    fireEvent.change(modeSelect, { target: { value: "toggle" } });
    await waitFor(
      () =>
        expect(invokeMock).toHaveBeenCalledWith(
          "save_app_config",
          expect.objectContaining({
            config: expect.objectContaining({
              hotkey_mode: "toggle",
              translate_target_lang: "en",
              insert_strategy: "auto",
              prefix_roles_enabled: true,
              qa_save_history: false,
              restore_clipboard: true,
            }),
          }),
        ),
      { timeout: 2500 },
    );
    // 首次加载的配置本身不触发保存（基线）。
    const saveCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "save_app_config");
    expect(saveCalls.length).toBe(1);
  });

  // R11：Windows UA 下显示 TSF 输入法状态卡（注册异常 → broken 文案 + 恢复按钮）。
  it("Windows 下渲染 TSF 状态卡与恢复按钮", async () => {
    // IS_WIN 是模块加载时常量 → 先 mock UA 再 resetModules 动态导入。
    const originalUA = navigator.userAgent;
    Object.defineProperty(navigator, "userAgent", {
      value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) jsdom",
      configurable: true,
    });
    try {
      mockInvoke({
        get_config: defaultConfig,
        windows_ime_status: {
          status: "registrationBroken",
          dllPath: null,
          usingTsf: true,
        },
      });
      vi.resetModules();
      const { default: SettingsWin } = await import("./Settings");
      render(<SettingsWin />);
      expect(await screen.findByText("TSF 输入法上屏（实验）")).toBeTruthy();
      // 状态异步加载：等 broken 文案出现。
      expect(await screen.findByText(/已注册但系统未收录/)).toBeTruthy();
      expect(screen.getByText("恢复系统输入法")).toBeTruthy();
    } finally {
      Object.defineProperty(navigator, "userAgent", {
        value: originalUA,
        configurable: true,
      });
      vi.resetModules();
    }
  });
});
