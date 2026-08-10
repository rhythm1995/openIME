import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import History from "./History";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

describe("History", () => {
  beforeEach(() => vi.clearAllMocks());

  it("空列表显示空状态", async () => {
    invokeMock.mockResolvedValue([]);
    render(<History />);
    await waitFor(() => expect(screen.getByText("还没有录音记录")).toBeInTheDocument());
  });

  it("按天分组渲染 utterance 并支持删除当天", async () => {
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
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<History />);
    // utterance 文本渲染在 day-row 里
    const row = await screen.findByText("你好世界");
    expect(row.closest(".day-row")).toBeTruthy();
    // 有日期分组头与删除按钮
    expect(screen.getByTitle("删除当天")).toBeTruthy();

    // 删除当天 → 删除其下所有 session
    fireEvent.click(screen.getByTitle("删除当天"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("delete_session", { sessionId: "s1" })
    );
  });
});
