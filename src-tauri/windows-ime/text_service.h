// CTextService：TSF 文本服务主体。
//
// 生命周期：宿主 ActivateProfile(我们的 profile) → 目标应用 UI 线程 CoCreate 本类 →
// ITfTextInputProcessor::Activate（此刻记录焦点上下文、建隐藏窗口 + 管道 server）→
// 宿主经管道 SubmitText → 隐藏窗口线程 RequestEditSession 上屏 → 宿主还原 profile →
// Deactivate（反向清理）。
#pragma once
#include <msctf.h>
#include <windows.h>

#include <string>

#include "edit_session.h"
#include "ipc_server.h"

namespace openime {

// 隐藏消息窗口类名（激活线程收 WM_APP+1 提交任务）。
constexpr wchar_t kMsgWndClass[] = L"OpenImeTsfMsgWnd";

class CTextService : public ITfTextInputProcessorEx,
                     public ITfThreadMgrEventSink {
 public:
  // IUnknown
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  // ITfTextInputProcessor
  STDMETHODIMP Activate(ITfThreadMgr* ptim, TfClientId tid) override;
  STDMETHODIMP Deactivate() override;

  // ITfTextInputProcessorEx（无扩展行为，转发 Activate）
  STDMETHODIMP ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD dwFlags) override;

  // ITfThreadMgrEventSink（跟踪焦点文档，保持 m_pic 有效）。
  STDMETHODIMP OnInitDocumentMgr(ITfDocumentMgr* /*pdim*/) override { return S_OK; }
  STDMETHODIMP OnUninitDocumentMgr(ITfDocumentMgr* /*pdim*/) override { return S_OK; }
  STDMETHODIMP OnSetFocus(ITfDocumentMgr* pdim_focus,
                          ITfDocumentMgr* /*pdim_prev_focus*/) override;
  STDMETHODIMP OnPushContext(ITfContext* /*pic*/) override { return S_OK; }
  STDMETHODIMP OnPopContext(ITfContext* /*pic*/) override { return S_OK; }

  // 隐藏窗口 wndproc（激活线程）调用的提交入口。
  void HandleCommitFromWindow();

  static void RegisterWndClassOnce();

 private:
  void UpdateFocusContext(ITfDocumentMgr* dim);
  void ShutdownRuntime();

  LONG ref_ = 1;
  ITfThreadMgr* tim_ = nullptr;
  TfClientId client_id_ = TF_CLIENTID_NULL;
  ITfContext* pic_ = nullptr;  // 焦点文档 top context
  DWORD sink_cookie_ = TF_INVALID_COOKIE;
  HWND msg_wnd_ = nullptr;
  PipeServer* pipe_ = nullptr;
};

}  // namespace openime
