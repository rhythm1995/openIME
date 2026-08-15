// 命名管道 server（TIP 侧，运行在目标进程的专属线程）。
//
// 生命周期 = TSF 激活周期：CTextService::Activate 启动、Deactivate 停止。
// 管道名携带本进程 pid 与激活线程 tid（宿主按前台窗口 pid/tid 反查连接）。
// 安全：DACL 仅允许当前用户 SID（无 Everyone ACE），FILE_FLAG_FIRST_PIPE_INSTANCE
// 防同名抢注。角色不可反：TIP = server，openIME 宿主 = client（设计 FR-11.6）。
#pragma once
#include <windows.h>
#include <mutex>
#include <string>
#include <thread>

#include "common.h"

namespace openime {

struct CommitJob {
  std::string session_id;
  std::string text;  // UTF-8
};

enum class CommitStatus { kCommitted, kRejected, kFailed };
enum class CommitError { kNone, kTimeout, kNoDocument, kRejected, kTooLarge, kProtocol };

struct CommitResult {
  CommitStatus status = CommitStatus::kFailed;
  CommitError error = CommitError::kNone;
};

// 宿主提交 → 转发到激活线程（隐藏窗口）执行 edit session 的桥。
class PipeServer {
 public:
  // owner_hwnd：激活线程上的隐藏消息窗口；PostMessage 失败/超时 → failed/timeout。
  PipeServer(HWND owner_hwnd, DWORD owner_tid);
  ~PipeServer();

  void Start();
  void Stop();  // 幂等；断开管道并 join 线程

  // 激活线程（隐藏窗口 wndproc）调用：取走 job → 返回是否有效。
  bool TakeJob(CommitJob* job);
  // 激活线程回写结果并唤醒管道线程。
  void CompleteJob(const CommitResult& result);

 private:
  void Run();
  bool ServeOneClient(HANDLE pipe);
  bool WriteLine(HANDLE pipe, const std::string& line);
  // 阻塞读一行（内部按 stop 事件可中断）；EOF/错误/超时返回 false。
  bool ReadLine(HANDLE pipe, std::string* line, uint32_t timeout_ms);

  HWND owner_hwnd_;
  DWORD owner_tid_;
  HANDLE stop_evt_ = nullptr;
  std::thread thread_;

  std::mutex job_mu_;
  CommitJob job_;              // 管道线程 → 激活线程
  HANDLE job_ready_ = nullptr; // 新 job 就绪（未用，PostMessage 已同步投递；保留结构对称）
  HANDLE job_done_ = nullptr;  // 激活线程完成
  CommitResult result_;
};

}  // namespace openime
