import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import History from "./History";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

describe("History", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
    // jsdom 无 ResizeObserver
    global.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  });

  it("空列表显示空状态", async () => {
    invokeMock.mockResolvedValue([]);
    render(<History />);
    await waitFor(() => expect(screen.getByText("还没有录音记录")).toBeInTheDocument());
  });

  it("按天分组渲染 utterance；三点菜单确认后删除当天", async () => {
    const sessions = [
      {
        id: "s1",
        title: "第一次录音",
        started_at: "2026-08-09T12:00:00Z",
        ended_at: null,
        engine: "cloud",
        provider: "bailian",
        model: "fun-asr-realtime",
      },
    ];
    const utterances = [
      {
        id: "u1",
        session_id: "s1",
        seq: 1,
        final_text: "你好世界",
        audio_path: null,
        created_at: "2026-08-09T12:00:01Z",
      },
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_sessions") return Promise.resolve(sessions);
      if (cmd === "list_utterances") return Promise.resolve(utterances);
      if (cmd === "delete_session") return Promise.resolve();
      return Promise.resolve(undefined);
    });

    render(<History />);
    const row = await screen.findByText("你好世界");
    expect(row.closest(".day-row")).toBeTruthy();

    // 三点打开菜单，不直接删除
    fireEvent.click(screen.getByTitle("更多"));
    expect(screen.getByRole("menuitem", { name: /删除/ })).toBeTruthy();
    expect(invokeMock).not.toHaveBeenCalledWith("delete_session", expect.anything());

    // 菜单「删除」→ 确认框
    fireEvent.click(screen.getByRole("menuitem", { name: /删除/ }));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText(/确定删除/)).toBeTruthy();

    // 确认删除
    fireEvent.click(screen.getByRole("button", { name: "删除" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("delete_session", { sessionId: "s1" })
    );
    await waitFor(() => expect(screen.getByText("已删除")).toBeTruthy());
  });

  it("右侧复制按钮写入剪贴板并提示已复制", async () => {
    const sessions = [
      {
        id: "s1",
        title: "第一次录音",
        started_at: "2026-08-09T12:00:00Z",
        ended_at: null,
        engine: "cloud",
        provider: "bailian",
        model: "fun-asr-realtime",
      },
    ];
    const utterances = [
      {
        id: "u1",
        session_id: "s1",
        seq: 1,
        final_text: "复制这段文字",
        audio_path: null,
        created_at: "2026-08-09T12:00:01Z",
      },
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_sessions") return Promise.resolve(sessions);
      if (cmd === "list_utterances") return Promise.resolve(utterances);
      return Promise.resolve(undefined);
    });

    render(<History />);
    await screen.findByText("复制这段文字");
    fireEvent.click(screen.getByLabelText("复制"));
    await waitFor(() =>
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("复制这段文字")
    );
    expect(screen.getByText("已复制")).toBeTruthy();
  });
});
