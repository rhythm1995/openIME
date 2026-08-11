import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import App from "./App";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: vi.fn().mockResolvedValue(undefined) }),
}));

const bailianConfig = {
  active_provider: 0,
  providers: [
    {
      kind: "bailian",
      base_url: "wss://x.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference",
      api_key: "sk-xxx",
      model: "fun-asr-realtime",
    },
  ],
  hotkey: "Alt+Shift+D",
  mute_other_audio: false,
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

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("渲染侧边栏与设置页", async () => {
    mockInvoke({
      ping: "在线",
      get_config: bailianConfig,
      check_permission: { kind: "accessibility", state: "granted", hint: "" },
      list_hotwords: [],
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("设置")).toBeInTheDocument());
    expect(screen.getByText("openIME")).toBeInTheDocument();
    expect(screen.getByText("已就绪")).toBeInTheDocument();
    // 方案 C：不再有顶部授权横幅
    expect(screen.queryByText(/需要授权才能完整使用/)).not.toBeInTheDocument();
  });

  it("侧边栏可切换到词典页", async () => {
    mockInvoke({
      ping: "ok",
      get_config: bailianConfig,
      check_permission: { kind: "accessibility", state: "granted", hint: "" },
      list_hotwords: [],
    });
    render(<App />);
    fireEvent.click(screen.getByText("词典"));
    await waitFor(() =>
      expect(screen.getByText(/自定义术语/)).toBeInTheDocument()
    );
  });

  it("侧边栏可切换到历史页", async () => {
    mockInvoke({
      ping: "ok",
      get_config: bailianConfig,
      check_permission: { kind: "accessibility", state: "granted", hint: "" },
      list_sessions: [],
    });
    render(<App />);
    fireEvent.click(screen.getByText("历史记录"));
    await waitFor(() => expect(screen.getByText("还没有录音记录")).toBeInTheDocument());
  });

  it("未授权时不显示顶部横幅，设置页仍有系统权限", async () => {
    mockInvoke({
      ping: "ok",
      get_config: bailianConfig,
      check_permission: { kind: "accessibility", state: "denied", hint: "" },
      list_hotwords: [],
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("系统权限")).toBeInTheDocument());
    expect(screen.queryByText(/需要授权才能完整使用/)).not.toBeInTheDocument();
  });
});
