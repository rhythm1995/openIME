import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import Settings from "./Settings";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
// 捕获每个事件的 handler，供测试手动触发（下载进度等）。
const listenHandlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    listenHandlers.set(event, handler);
    return Promise.resolve(() => {});
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

/** 触发一次后端事件（如 model://download-progress）。 */
function emitBackend(event: string, payload: unknown) {
  listenHandlers.get(event)?.({ payload });
}

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
  polish_local_model: "qwen3.5-2b",
  polish_cloud_model: "qwen-turbo",
  polish_cloud_protocol: "openai_chat",
  polish_cloud_endpoint: "",
  polish_cloud_api_key: "",
  local_asr_model: "sensevoice",
  // P1 新字段（PR2/PR4/PR5/PR6 默认值）。
  translate_hotkey: null,
  translate_target_lang: "en",
  translate_with_polish: false,
  // 本地三件套新字段。
  translate_local_model: "milmmt-1b",
  translate_use_llm_fallback: false,
  translate_policy: "prefer_cloud",
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
    listenHandlers.clear();
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

  // ── 本地三件套 ──

  it("AI 视图渲染本地模型卡片：打开目录 / 预算条 / 兼译开关 / 策略下拉", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [
        {
          id: "qwen3.5-2b",
          kind: "polish",
          title: "均衡 · Qwen3.5-2B",
          description: "离线润色 · 约 1.4GB · 16GB 默认。",
          approx_size: 1_396_198_496,
          installed: false,
          active: true,
          missing_size: 1_396_198_496,
          perf_tag: { tag: "适合", kind: "suitable", reason: "ok", color: "var(--success)" },
          recommended: true,
          arch: "qwen35",
        },
      ],
      list_local_translate_models: [],
      get_model_suite_info: {
        model_root: "/tmp/models",
        budget_bytes: 10 * 1024 * 1024 * 1024,
        used_bytes: 3 * 1024 * 1024 * 1024,
        has_cloud: false,
        weak_machine: false,
        llm_feature: true,
      },
    });
    render(<Settings view="ai" />);
    expect(await screen.findByText("本地模型")).toBeTruthy();
    expect(screen.getByText("打开模型目录")).toBeTruthy();
    expect(await screen.findByText("用润色模型兼做翻译")).toBeTruthy();
    expect(screen.getByText("翻译路由策略")).toBeTruthy();
    expect(await screen.findByText(/均衡 · Qwen3\.5-2B/)).toBeTruthy();
    expect(await screen.findByText(/预算/)).toBeTruthy();
  });

  it("打开模型目录按钮调用 open_model_directory", async () => {
    mockInvoke({
      get_config: defaultConfig,
      open_model_directory: "/tmp/models",
    });
    render(<Settings view="ai" />);
    const btn = await screen.findByRole("button", { name: /打开模型目录/ });
    fireEvent.click(btn);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_model_directory"),
    );
  });

  // ── 快捷键捕获 ──

  it("录音快捷键：点击后按键直接捕获并写入配置（挂起真实快捷键）", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    await screen.findByText("录音快捷键");
    const btn = screen.getByRole("button", { name: "Fn" });
    fireEvent.click(btn);
    // 捕获态：后端挂起真实快捷键。
    expect(invokeMock).toHaveBeenCalledWith("set_capture_suspend", { suspend: true });
    // 按下 Ctrl+Alt+T。
    fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
    fireEvent.keyDown(window, { key: "t", ctrlKey: true, altKey: true });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("save_app_config", expect.objectContaining({
        config: expect.objectContaining({ hotkey: "Ctrl+Alt+T" }),
      })),
      { timeout: 2500 },
    );
    // 捕获结束：恢复快捷键。
    expect(invokeMock).toHaveBeenCalledWith("set_capture_suspend", { suspend: false });
  });

  it("翻译/QA 快捷键为可选：清除按钮置空", async () => {
    mockInvoke({
      get_config: {
        ...defaultConfig,
        translate_hotkey: "Alt+Shift+T",
      },
    });
    render(<Settings />);
    await screen.findByText("翻译快捷键（可选）");
    const clear = screen.getByTitle("清除");
    fireEvent.click(clear);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("save_app_config", expect.objectContaining({
        config: expect.objectContaining({ translate_hotkey: null }),
      })),
      { timeout: 2500 },
    );
  });

  it("快捷键捕获：Esc 取消，值不变且不保存", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    await screen.findByText("录音快捷键");
    fireEvent.click(screen.getByRole("button", { name: "Fn" }));
    expect(invokeMock).toHaveBeenCalledWith("set_capture_suspend", { suspend: true });
    fireEvent.keyDown(window, { key: "Escape" });
    // Esc 退出捕获并恢复挂起；不产生任何保存。
    expect(invokeMock).toHaveBeenCalledWith("set_capture_suspend", { suspend: false });
    const saves = invokeMock.mock.calls.filter((c) => c[0] === "save_app_config");
    expect(saves).toHaveLength(0);
  });

  it("快捷键捕获：录音键允许 CapsLock 单键；组合键字段忽略", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    await screen.findByText("录音快捷键");
    // 录音键 allowSingle=true → CapsLock 直接生效。
    fireEvent.click(screen.getByRole("button", { name: "Fn" }));
    fireEvent.keyDown(window, { key: "CapsLock" });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("save_app_config", expect.objectContaining({
        config: expect.objectContaining({ hotkey: "CapsLock" }),
      })),
      { timeout: 2500 },
    );
  });

  it("快捷键捕获：组合键字段对 CapsLock 不保存；仅修饰键显示半成品预览", async () => {
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    await screen.findByText("翻译快捷键（可选）");
    // 翻译/QA 两个可选快捷键都未设置：取第一个（翻译）。
    const captureBtn = screen.getAllByRole("button", { name: /未设置/ })[0];
    fireEvent.click(captureBtn);
    // CapsLock 在组合键字段被忽略（不保存、仍处捕获态）。
    fireEvent.keyDown(window, { key: "CapsLock" });
    // 只按 Ctrl：按钮显示 "Ctrl+…" 半成品，不保存。
    fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
    expect(screen.getByRole("button", { name: "Ctrl+…" })).toBeTruthy();
    const saves = invokeMock.mock.calls.filter((c) => c[0] === "save_app_config");
    expect(saves).toHaveLength(0);
    fireEvent.keyDown(window, { key: "Escape" });
  });

  // ── LLM 卡片操作：下载 / 启用 / 删除 ──

  const polishEntry = (over: Partial<Record<string, unknown>>): Record<string, unknown> => ({
    id: "qwen3.5-2b",
    kind: "polish",
    title: "均衡 · Qwen3.5-2B",
    description: "d",
    approx_size: 1_396_198_496,
    installed: false,
    active: false,
    missing_size: 1_396_198_496,
    perf_tag: { tag: "适合", kind: "suitable", reason: "", color: "var(--success)" },
    recommended: false,
    arch: "qwen35",
    ...over,
  });

  it("未安装卡片：点下载调用 install_llm_model", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [polishEntry({})],
    });
    render(<Settings view="ai" />);
    const btn = await screen.findByRole("button", { name: "下载" });
    fireEvent.click(btn);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("install_llm_model", { id: "qwen3.5-2b" }),
    );
  });

  it("已安装卡片：点启用调用 set_active_polish_model", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [polishEntry({ installed: true })],
    });
    const { container } = render(<Settings view="ai" />);
    await screen.findByText(/均衡 · Qwen3\.5-2B/);
    // 「启用」按钮在润色卡与翻译 none 卡各有一个：限定本卡范围。
    const card = container.querySelector('[data-model-id="qwen3.5-2b"]')!;
    const enable = Array.from(card.querySelectorAll("button")).find(
      (b) => b.textContent === "启用",
    ) as HTMLButtonElement;
    fireEvent.click(enable);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_active_polish_model", { modelId: "qwen3.5-2b" }),
    );
  });

  it("删除模型：confirm 取消不调用；确认后调用 delete_llm_model", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [polishEntry({ installed: true })],
    });
    render(<Settings view="ai" />);
    await screen.findByText(/均衡 · Qwen3\.5-2B/);
    const del = screen.getByTitle("删除该模型（释放磁盘）");
    fireEvent.click(del);
    expect(confirmSpy).toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "delete_llm_model",
      expect.anything(),
    );
    confirmSpy.mockReturnValue(true);
    fireEvent.click(del);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("delete_llm_model", { id: "qwen3.5-2b" }),
    );
    confirmSpy.mockRestore();
  });

  it("下载进度事件挂到对应卡片：按钮变下载中", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [polishEntry({})],
    });
    render(<Settings view="ai" />);
    await screen.findByRole("button", { name: "下载" });
    // 后端推进度（target_id 指向该卡）→ 按钮进入「下载中…」态。
    emitBackend("model://download-progress", {
      phase: "downloading",
      target_id: "qwen3.5-2b",
      total_downloaded: 500,
      total_size: 1000,
      speed_bps: 1024,
    });
    expect(await screen.findByRole("button", { name: "下载中…" })).toBeTruthy();
  });

  // ── 策略 / 兼译变更保存 ──

  it("翻译策略下拉变更后防抖保存 prefer_local", async () => {
    mockInvoke({ get_config: defaultConfig, list_style_packs: [] });
    render(<Settings view="ai" />);
    await screen.findByText("翻译路由策略");
    const policy = screen
      .getByText("翻译路由策略")
      .closest(".field")!
      .querySelector("select") as HTMLSelectElement;
    fireEvent.change(policy, { target: { value: "prefer_local" } });
    await waitFor(
      () =>
        expect(invokeMock).toHaveBeenCalledWith(
          "save_app_config",
          expect.objectContaining({
            config: expect.objectContaining({ translate_policy: "prefer_local" }),
          }),
        ),
      { timeout: 2500 },
    );
  });

  it("兼译开关打开后防抖保存 translate_use_llm_fallback", async () => {
    mockInvoke({ get_config: defaultConfig, list_style_packs: [] });
    render(<Settings view="ai" />);
    await screen.findByText("用润色模型兼做翻译");
    const toggle = screen
      .getByText("用润色模型兼做翻译")
      .closest(".set-row")!
      .querySelector('input[type="checkbox"]') as HTMLInputElement;
    fireEvent.click(toggle);
    await waitFor(
      () =>
        expect(invokeMock).toHaveBeenCalledWith(
          "save_app_config",
          expect.objectContaining({
            config: expect.objectContaining({ translate_use_llm_fallback: true }),
          }),
        ),
      { timeout: 2500 },
    );
  });

  // ── 模型排序与「不使用本地专翻」卡片 ──

  it("润色模型列表：机器推荐的档排第一", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_local_polish_models: [
        {
          id: "qwen3.5-0.8b",
          kind: "polish",
          title: "极速 · Qwen3.5-0.8B",
          description: "d",
          approx_size: 532_517_120,
          installed: false,
          active: false,
          missing_size: 532_517_120,
          perf_tag: { tag: "适合", kind: "suitable", reason: "", color: "var(--success)" },
          recommended: false,
          arch: "qwen35",
        },
        {
          id: "qwen3.5-2b",
          kind: "polish",
          title: "均衡 · Qwen3.5-2B",
          description: "d",
          approx_size: 1_396_198_496,
          installed: false,
          active: true,
          missing_size: 1_396_198_496,
          perf_tag: { tag: "适合", kind: "suitable", reason: "", color: "var(--success)" },
          recommended: true,
          arch: "qwen35",
        },
      ],
    });
    const { container } = render(<Settings view="ai" />);
    // 等列表真正渲染出来（静态标签不等待数据）。
    await screen.findByText(/均衡 · Qwen3\.5-2B/);
    const ids = Array.from(container.querySelectorAll("[data-model-id]")).map(
      (el) => el.getAttribute("data-model-id"),
    );
    // 推荐档（2B）排第一；「不使用本地专翻」卡片（none）在翻译区、随其后。
    expect(ids.indexOf("qwen3.5-2b")).toBeLessThan(ids.indexOf("qwen3.5-0.8b"));
  });

  it("不使用本地专翻：启用后绿色「无需安装」+ 已启用；弱机加推荐标", async () => {
    mockInvoke({
      get_config: { ...defaultConfig, translate_local_model: "" },
      get_model_suite_info: {
        model_root: "/tmp/models",
        budget_bytes: 10 * 1024 * 1024 * 1024,
        used_bytes: 2 * 1024 * 1024 * 1024,
        has_cloud: true,
        weak_machine: true,
        llm_feature: true,
      },
    });
    render(<Settings view="ai" />);
    expect(await screen.findByText("无需安装")).toBeTruthy();
    expect(screen.getByText("已启用")).toBeTruthy();
    // 弱机 → 该选项是推荐模型，带「推荐」徽标。
    expect(await screen.findByText("推荐")).toBeTruthy();
  });

  it("键位文案跟随当前录音键（改键位不误导）", async () => {
    // 默认 Fn：短按阈值提示按 Fn 表述。
    mockInvoke({ get_config: defaultConfig });
    render(<Settings />);
    await screen.findByText("录音快捷键");
    expect(screen.getByText(/按住 Fn 超过该时长才开录/)).toBeTruthy();
    // 功能测试卡提示也含 Fn。
    expect(screen.getByText(/按 Fn 开始录音/)).toBeTruthy();
  });

  it("改录音键后键位文案跟随新键", async () => {
    mockInvoke({ get_config: { ...defaultConfig, hotkey: "Ctrl+Alt+T" } });
    render(<Settings />);
    await screen.findByText("录音快捷键");
    expect(screen.getByText(/按住 Ctrl\+Alt\+T 超过该时长才开录/)).toBeTruthy();
    expect(screen.getByText(/按 Ctrl\+Alt\+T 开始录音/)).toBeTruthy();
    // 不再出现写死的 Fn 表述。
    expect(screen.queryByText(/按住 Fn 超过该时长才开录/)).toBeNull();
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
    // 组合触发说明与「跟随 AI 润色」。
    expect(screen.getByText(/小友翻译我想要走了/)).toBeTruthy();
    expect(screen.getByText(/与 AI 润色相同的模型/)).toBeTruthy();
    // 助手名称输入框默认值。
    expect(await screen.findByDisplayValue("小友")).toBeTruthy();
  });

  it("助手名称修改后防抖保存", async () => {
    mockInvoke({ get_config: defaultConfig, list_style_packs: [] });
    render(<Settings view="ai" />);
    const input = await screen.findByDisplayValue("小友");
    fireEvent.blur(input, { target: { value: "阿法" } });
    await waitFor(
      () =>
        expect(invokeMock).toHaveBeenCalledWith(
          "save_app_config",
          expect.objectContaining({
            config: expect.objectContaining({ assistant_name: "阿法" }),
          }),
        ),
      { timeout: 2500 },
    );
  });

  it("角色提供商下拉默认「跟随 AI 润色」", async () => {
    mockInvoke({
      get_config: defaultConfig,
      list_style_packs: [
        { id: "b-mail", name: "邮件助手", system_prompt: "写一封正式邮件", is_builtin: true, ord: 0, match_prefix: "邮件|mail", provider: null, model: null, role_kind: "default", output_mode: "insert" },
      ],
    });
    render(<Settings view="ai" />);
    expect(await screen.findByText(/邮件助手/)).toBeTruthy();
    const provider = await screen.findByDisplayValue("跟随 AI 润色（默认）");
    expect(provider).toBeTruthy();
    // 三选项：跟随 / 云端 / 本地。
    expect(screen.getByRole("option", { name: "云端" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "本地 GGUF" })).toBeTruthy();
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
});
