#include "ipc_server.h"

#include <aclapi.h>
#include <sddl.h>

namespace openime {
namespace {

// DACL：仅当前用户 SID GENERIC_ALL，无 Everyone（设计 FR-11.6）。
bool BuildCurrentUserSa(SECURITY_ATTRIBUTES* sa, SECURITY_DESCRIPTOR* sd,
                        PACL* acl_out) {
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return false;
  DWORD len = 0;
  GetTokenInformation(token, TokenUser, nullptr, 0, &len);
  if (len == 0) {
    CloseHandle(token);
    return false;
  }
  auto* user = static_cast<TOKEN_USER*>(HeapAlloc(GetProcessHeap(), 0, len));
  if (!user ||
      !GetTokenInformation(token, TokenUser, user, len, &len)) {
    if (user) HeapFree(GetProcessHeap(), 0, user);
    CloseHandle(token);
    return false;
  }
  CloseHandle(token);
  EXPLICIT_ACCESS_W ea{};
  ea.grfAccessPermissions = GENERIC_ALL;
  ea.grfAccessMode = GRANT_ACCESS;
  ea.grfInheritance = NO_INHERITANCE;
  ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
  ea.Trustee.ptstrName = reinterpret_cast<LPWSTR>(user->User.Sid);
  PACL acl = nullptr;
  if (SetEntriesInAclW(1, &ea, nullptr, &acl) != ERROR_SUCCESS) {
    HeapFree(GetProcessHeap(), 0, user);
    return false;
  }
  if (!InitializeSecurityDescriptor(sd, SECURITY_DESCRIPTOR_REVISION) ||
      !SetSecurityDescriptorDacl(sd, TRUE, acl, FALSE)) {
    LocalFree(acl);
    HeapFree(GetProcessHeap(), 0, user);
    return false;
  }
  // user SID 生命周期：DACL 里引用了该 SID 拷贝（SetEntriesInAcl 内部复制），
  // 此处即可释放。
  HeapFree(GetProcessHeap(), 0, user);
  sa->nLength = sizeof(*sa);
  sa->lpSecurityDescriptor = sd;
  sa->bInheritHandle = FALSE;
  *acl_out = acl;
  return true;
}

std::string MakeSubmitResult(const std::string& session_id, CommitStatus st,
                             CommitError err) {
  const char* status = st == CommitStatus::kCommitted ? "committed"
                     : st == CommitStatus::kRejected   ? "rejected"
                                                       : "failed";
  std::string err_lit = "null";
  if (err != CommitError::kNone) {
    const char* e = err == CommitError::kTimeout      ? "timeout"
                  : err == CommitError::kNoDocument   ? "no_document"
                  : err == CommitError::kRejected     ? "rejected"
                  : err == CommitError::kTooLarge     ? "too_large"
                                                      : "protocol";
    err_lit = std::string("\"") + e + "\"";
  }
  return "{\"type\":\"submitResult\",\"protocolVersion\":" +
         std::to_string(kProtocolVersion) + ",\"sessionId\":\"" +
         JsonEscape(session_id) + "\",\"status\":\"" + status +
         "\",\"errorCode\":" + err_lit + "}\n";
}

}  // namespace

PipeServer::PipeServer(HWND owner_hwnd, DWORD owner_tid)
    : owner_hwnd_(owner_hwnd), owner_tid_(owner_tid) {}

PipeServer::~PipeServer() { Stop(); }

void PipeServer::Start() {
  stop_evt_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  job_done_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);  // auto-reset
  thread_ = std::thread(&PipeServer::Run, this);
}

void PipeServer::Stop() {
  if (stop_evt_) SetEvent(stop_evt_);
  if (thread_.joinable()) thread_.join();
  if (stop_evt_) {
    CloseHandle(stop_evt_);
    stop_evt_ = nullptr;
  }
  if (job_done_) {
    CloseHandle(job_done_);
    job_done_ = nullptr;
  }
}

bool PipeServer::TakeJob(CommitJob* job) {
  std::lock_guard<std::mutex> lk(job_mu_);
  *job = job_;
  return !job->session_id.empty();
}

void PipeServer::CompleteJob(const CommitResult& result) {
  {
    std::lock_guard<std::mutex> lk(job_mu_);
    result_ = result;
  }
  if (job_done_) SetEvent(job_done_);
}

// ── 管道线程主体 ──

void PipeServer::Run() {
  const std::wstring name = PipeNameForCurrentThread();
  for (;;) {
    SECURITY_ATTRIBUTES sa{};
    SECURITY_DESCRIPTOR sd;
    PACL acl = nullptr;
    const bool have_sa = BuildCurrentUserSa(&sa, &sd, &acl);
    // OVERLAPPED：Connect/Read/Write 全部可被 stop_evt 打断（Deactivate 不许卡死）。
    HANDLE pipe = CreateNamedPipeW(
        name.c_str(),
        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        1, static_cast<DWORD>(kMaxTextBytes * 2),
        static_cast<DWORD>(kMaxTextBytes * 2), 0,
        have_sa ? &sa : nullptr);
    if (have_sa && acl) LocalFree(acl);
    if (pipe == INVALID_HANDLE_VALUE) {
      // 管道名被占（异常残留）或资源不足：放弃本线程（宿主侧 800ms 超时兜底）。
      return;
    }
    OVERLAPPED ov{};
    ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    ConnectNamedPipe(pipe, &ov);
    HANDLE waits[2] = {ov.hEvent, stop_evt_};
    DWORD w = WaitForMultipleObjects(2, waits, FALSE, INFINITE);
    if (w != WAIT_OBJECT_0) {
      // stop 或异常：关闭句柄即断开未完成连接。
      CloseHandle(ov.hEvent);
      CloseHandle(pipe);
      return;
    }
    bool client_ok = ServeOneClient(pipe);
    FlushFileBuffers(pipe);
    DisconnectNamedPipe(pipe);
    CloseHandle(pipe);
    CloseHandle(ov.hEvent);
    if (!client_ok) {
      // stop 打断才退出；普通 EOF/坏帧继续服务下一个宿主连接。
      if (WaitForSingleObject(stop_evt_, 0) == WAIT_OBJECT_0) return;
    }
  }
}

bool PipeServer::WriteLine(HANDLE pipe, const std::string& line) {
  OVERLAPPED ov{};
  ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  DWORD written = 0;
  bool ok = WriteFile(pipe, line.data(), static_cast<DWORD>(line.size()), nullptr,
                      &ov) ||
             GetLastError() == ERROR_IO_PENDING;
  if (ok) {
    HANDLE waits[2] = {ov.hEvent, stop_evt_};
    ok = WaitForMultipleObjects(2, waits, FALSE, 5000) == WAIT_OBJECT_0;
    if (ok && !GetOverlappedResult(pipe, &ov, &written, FALSE)) ok = false;
  }
  CloseHandle(ov.hEvent);
  return ok && written == line.size();
}

bool PipeServer::ReadLine(HANDLE pipe, std::string* line, uint32_t timeout_ms) {
  // 行缓冲按需累积；每轮最多读 8 KiB。
  static thread_local std::string pending;
  size_t nl = pending.find('\n');
  if (nl != std::string::npos) {
    *line = pending.substr(0, nl);
    pending.erase(0, nl + 1);
    return true;
  }
  char buf[8192];
  OVERLAPPED ov{};
  ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  ULONGLONG deadline = GetTickCount64() + timeout_ms;
  for (;;) {
    DWORD got = 0;
    BOOL r = ReadFile(pipe, buf, sizeof(buf), nullptr, &ov);
    if (!r && GetLastError() != ERROR_IO_PENDING) break;  // EOF / 断开
    ULONGLONG now = GetTickCount64();
    if (now >= deadline) {
      CancelIo(pipe);
      CloseHandle(ov.hEvent);
      return false;
    }
    HANDLE waits[2] = {ov.hEvent, stop_evt_};
    DWORD w = WaitForMultipleObjects(
        2, waits, FALSE, static_cast<DWORD>(deadline - now));
    if (w != WAIT_OBJECT_0) {
      CancelIo(pipe);
      CloseHandle(ov.hEvent);
      return false;
    }
    if (!GetOverlappedResult(pipe, &ov, &got, FALSE)) break;
    CloseHandle(ov.hEvent);
    pending.append(buf, buf + got);
    // 超限防御：坏客户端不允许无限堆积。
    if (pending.size() > kMaxTextBytes * 2 + 4096) return false;
    nl = pending.find('\n');
    if (nl != std::string::npos) {
      *line = pending.substr(0, nl);
      pending.erase(0, nl + 1);
      return true;
    }
    ov = {};
    ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  }
  CloseHandle(ov.hEvent);
  return false;
}

bool PipeServer::ServeOneClient(HANDLE pipe) {
  // 连接建立即宣告就绪（宿主以 clientReady 为激活成功的唯一标准，FR-11.4/KD-10）。
  char ready[160];
  sprintf_s(ready,
            "{\"type\":\"clientReady\",\"protocolVersion\":%u,\"processId\":%lu,"
            "\"threadId\":%lu}\n",
            kProtocolVersion,
            static_cast<unsigned long>(GetCurrentProcessId()),
            static_cast<unsigned long>(owner_tid_));
  if (!WriteLine(pipe, ready)) return false;

  std::string line;
  while (ReadLine(pipe, &line, kCommitTimeoutMs * 4)) {
    std::string type;
    if (!JsonFindString(line, "type", type)) {
      WriteLine(pipe, MakeSubmitResult("", CommitStatus::kFailed, CommitError::kProtocol));
      continue;
    }
    if (type == "ping") {
      WriteLine(pipe, std::string("{\"type\":\"ping\",\"protocolVersion\":") +
                          std::to_string(kProtocolVersion) + "}\n");
      continue;
    }
    if (type != "submitText") {
      WriteLine(pipe, MakeSubmitResult("", CommitStatus::kFailed, CommitError::kProtocol));
      continue;
    }
    CommitJob job;
    uint32_t ver = 0;
    JsonFindNumber(line, "protocolVersion", ver);
    if (ver != kProtocolVersion || !JsonFindString(line, "sessionId", job.session_id) ||
        !JsonFindString(line, "text", job.text)) {
      WriteLine(pipe, MakeSubmitResult("", CommitStatus::kFailed, CommitError::kProtocol));
      continue;
    }
    if (job.text.size() > kMaxTextBytes) {
      WriteLine(pipe, MakeSubmitResult(job.session_id, CommitStatus::kFailed,
                                       CommitError::kTooLarge));
      continue;
    }
    // 转发到激活线程（隐藏窗口）；宿主在管道上等 submitResult。
    ResetEvent(job_done_);
    {
      std::lock_guard<std::mutex> lk(job_mu_);
      job_ = job;
      result_ = CommitResult{CommitStatus::kFailed, CommitError::kTimeout};
    }
    if (PostMessageW(owner_hwnd_, WM_APP + 1, 0, 0)) {
      WaitForSingleObject(job_done_, kCommitTimeoutMs);
    }
    CommitResult r;
    {
      std::lock_guard<std::mutex> lk(job_mu_);
      r = result_;
      job_ = CommitJob{};  // 清空，防 stale 重放
    }
    if (!WriteLine(pipe, MakeSubmitResult(job.session_id, r.status, r.error)))
      return false;
  }
  return true;
}

}  // namespace openime
