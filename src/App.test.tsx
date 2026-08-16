import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import App from "./App";
import i18n from "./i18n";

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
const openUrlMock = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrlMock(...args),
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
    await waitFor(() => expect(screen.getByText("语音配置")).toBeInTheDocument());
    expect(screen.getByText("AI 增强配置")).toBeInTheDocument();
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

  it("点击左下角意见反馈会打开 GitHub Issues", async () => {
    mockInvoke({
      ping: "ok",
      get_config: bailianConfig,
      check_permission: { kind: "accessibility", state: "granted", hint: "" },
      list_hotwords: [],
    });
    render(<App />);
    fireEvent.click(screen.getByText("意见反馈"));
    await waitFor(() =>
      expect(openUrlMock).toHaveBeenCalledWith("https://github.com/rhythm1995/openIME/issues/new")
    );
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

  // ── TDD5：语言切换 → 后端 ui_lang 下发 + 默认助手名跟随（小友↔IME） ──

  it("切 en：set_ui_lang 下发，默认助手名 小友→IME 并保存，UI 随之切换", async () => {
    const saved: { assistant_name?: string }[] = [];
    invokeMock.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === "get_config")
        return Promise.resolve({ ...bailianConfig, assistant_name: "小友" });
      if (cmd === "save_app_config") {
        saved.push((args as { config: { assistant_name?: string } }).config);
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    await act(async () => {
      await i18n.changeLanguage("zh");
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("语音配置")).toBeInTheDocument());
    fireEvent.click(screen.getByTitle("切换界面语言"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_ui_lang", { lang: "en" }),
    );
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].assistant_name).toBe("IME");
    await waitFor(() => expect(screen.getByText("Voice")).toBeInTheDocument());
  });

  it("切 en：自定义助手名不动，不触发配置保存", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_config")
        return Promise.resolve({ ...bailianConfig, assistant_name: "小明" });
      return Promise.resolve(undefined);
    });
    await act(async () => {
      await i18n.changeLanguage("zh");
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("语音配置")).toBeInTheDocument());
    fireEvent.click(screen.getByTitle("切换界面语言"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_ui_lang", { lang: "en" }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "save_app_config",
      expect.anything(),
    );
  });

  it("切回 zh：默认助手名 IME→小友", async () => {
    const saved: { assistant_name?: string }[] = [];
    invokeMock.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === "get_config")
        return Promise.resolve({ ...bailianConfig, assistant_name: "IME" });
      if (cmd === "save_app_config") {
        saved.push((args as { config: { assistant_name?: string } }).config);
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    await act(async () => {
      await i18n.changeLanguage("en");
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("Voice")).toBeInTheDocument());
    fireEvent.click(screen.getByTitle("Switch interface language"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_ui_lang", { lang: "zh" }),
    );
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].assistant_name).toBe("小友");
    await waitFor(() => expect(screen.getByText("语音配置")).toBeInTheDocument());
  });

  it("启动即 EN（老版本升级）：自动同步后端 ui_lang 并把默认助手名换为 IME", async () => {
    // 老配置：无 ui_lang 字段（后端默认 zh）、助手名还是默认 小友。
    const saved: { assistant_name?: string }[] = [];
    invokeMock.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === "get_config")
        return Promise.resolve({ ...bailianConfig, assistant_name: "小友" });
      if (cmd === "save_app_config") {
        saved.push((args as { config: { assistant_name?: string } }).config);
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    await act(async () => {
      await i18n.changeLanguage("en");
    });
    render(<App />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_ui_lang", { lang: "en" }),
    );
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].assistant_name).toBe("IME");
    await waitFor(() => expect(screen.getByText("Voice")).toBeInTheDocument());
  });
});
