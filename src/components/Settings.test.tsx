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

const defaultConfig = {
  active_provider: 0,
  providers: [
    { kind: "sherpa", base_url: "", api_key: "", model: "sensevoice" },
  ],
  hotkey: "Fn",
  mute_other_audio: false,
  polish_enabled: false,
  polish_mode: "off",
  polish_policy: "prefer_local",
  polish_cloud_model: "qwen-turbo",
  polish_cloud_protocol: "openai_chat",
  polish_cloud_endpoint: "",
  polish_cloud_api_key: "",
  local_asr_model: "sensevoice",
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

/** 第一个 select 是引擎类型（sherpa/zh/toggle 等字段在它后面）。 */
function engineSelect(container: HTMLElement): HTMLSelectElement {
  return container.querySelector("select") as HTMLSelectElement;
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
    render(<Settings />);
    for (const label of ["保持原样", "中度润色", "高度润色"]) {
      expect(await screen.findByText(label)).toBeTruthy();
    }
  });

  it("润色开着时显示 LLM 协议选择", async () => {
    mockInvoke({
      get_config: { ...defaultConfig, polish_mode: "light" },
    });
    render(<Settings />);
    expect(await screen.findByText("云端 LLM 协议")).toBeTruthy();
    expect(screen.getByText("OpenAI Chat Completions（/chat/completions）")).toBeTruthy();
    expect(screen.getByText("Anthropic Messages（/v1/messages）")).toBeTruthy();
    expect(screen.getByText("OpenAI Responses（/v1/responses）")).toBeTruthy();
  });
});
