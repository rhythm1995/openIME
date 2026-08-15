#include "text_service.h"

namespace openime {

namespace {

// 隐藏窗口：跑在激活线程（= 目标应用 UI 线程），保证 RequestEditSession 与
// edit cookie 的线程模型正确（设计风险表：必须经线程转发，不能在管道线程直调）。
LRESULT CALLBACK MsgWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
  if (msg == WM_APP + 1) {
    auto* self = reinterpret_cast<CTextService*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    if (self) self->HandleCommitFromWindow();
    return 0;
  }
  return DefWindowProcW(hwnd, msg, wp, lp);
}

}  // namespace

void CTextService::RegisterWndClassOnce() {
  static bool done = [] {
    WNDCLASSW wc{};
    wc.lpfnWndProc = MsgWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = kMsgWndClass;
    // 已注册（同进程二次激活）时失败可忽略。
    RegisterClassW(&wc);
    return true;
  }();
  (void)done;
}

STDMETHODIMP CTextService::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_INVALIDARG;
  if (IsEqualIID(riid, IID_IUnknown) ||
      IsEqualIID(riid, IID_ITfTextInputProcessor) ||
      IsEqualIID(riid, IID_ITfTextInputProcessorEx)) {
    *ppv = static_cast<ITfTextInputProcessorEx*>(this);
  } else if (IsEqualIID(riid, IID_ITfThreadMgrEventSink)) {
    *ppv = static_cast<ITfThreadMgrEventSink*>(this);
  } else {
    *ppv = nullptr;
    return E_NOINTERFACE;
  }
  AddRef();
  return S_OK;
}
STDMETHODIMP_(ULONG) CTextService::AddRef() {
  return InterlockedIncrement(&ref_);
}
STDMETHODIMP_(ULONG) CTextService::Release() {
  LONG r = InterlockedDecrement(&ref_);
  if (r == 0) delete this;
  return static_cast<ULONG>(r);
}

STDMETHODIMP CTextService::Activate(ITfThreadMgr* ptim, TfClientId tid) {
  tim_ = ptim;
  tim_->AddRef();
  client_id_ = tid;

  // 焦点跟踪：经 ITfSource::AdviseSink（ITfThreadMgr 不直接暴露事件接口）。
  ITfSource* source = nullptr;
  if (SUCCEEDED(tim_->QueryInterface(IID_ITfSource,
                                     reinterpret_cast<void**>(&source))) &&
      source) {
    source->AdviseSink(IID_ITfThreadMgrEventSink,
                       static_cast<ITfThreadMgrEventSink*>(this), &sink_cookie_);
    source->Release();
  }
  ITfDocumentMgr* dim = nullptr;
  if (SUCCEEDED(tim_->GetFocus(&dim)) && dim) {
    UpdateFocusContext(dim);
    dim->Release();
  }

  RegisterWndClassOnce();
  msg_wnd_ = CreateWindowExW(0, kMsgWndClass, L"", 0, 0, 0, 0, 0, HWND_MESSAGE,
                             nullptr, GetModuleHandleW(nullptr), nullptr);
  if (msg_wnd_) {
    SetWindowLongPtrW(msg_wnd_, GWLP_USERDATA,
                      reinterpret_cast<LONG_PTR>(this));
  }

  pipe_ = new PipeServer(msg_wnd_, GetCurrentThreadId());
  pipe_->Start();
  OutputDebugStringW(L"[openIME TSF] Activate：pipe server 已启动");
  return S_OK;
}

STDMETHODIMP CTextService::ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD) {
  return Activate(ptim, tid);
}

STDMETHODIMP CTextService::Deactivate() {
  ShutdownRuntime();
  return S_OK;
}

STDMETHODIMP CTextService::OnSetFocus(ITfDocumentMgr* pdim_focus,
                                      ITfDocumentMgr* /*pdim_prev_focus*/) {
  UpdateFocusContext(pdim_focus);
  return S_OK;
}

void CTextService::UpdateFocusContext(ITfDocumentMgr* dim) {
  if (!dim) return;
  ITfContext* pic = nullptr;
  if (SUCCEEDED(dim->GetTop(&pic)) && pic) {
    if (pic_) pic_->Release();
    pic_ = pic;  // 接管 GetTop 的引用
  }
}

void CTextService::ShutdownRuntime() {
  if (pipe_) {
    pipe_->Stop();
    delete pipe_;
    pipe_ = nullptr;
  }
  if (msg_wnd_) {
    // Deactivate 与 Activate 同线程（TSF 保证），DestroyWindow 安全。
    SetWindowLongPtrW(msg_wnd_, GWLP_USERDATA, 0);
    DestroyWindow(msg_wnd_);
    msg_wnd_ = nullptr;
  }
  if (sink_cookie_ != TF_INVALID_COOKIE && tim_) {
    ITfSource* source = nullptr;
    if (SUCCEEDED(tim_->QueryInterface(IID_ITfSource,
                                       reinterpret_cast<void**>(&source))) &&
        source) {
      source->UnadviseSink(sink_cookie_);
      source->Release();
    }
    sink_cookie_ = TF_INVALID_COOKIE;
  }
  if (pic_) {
    pic_->Release();
    pic_ = nullptr;
  }
  if (tim_) {
    tim_->Release();
    tim_ = nullptr;
  }
  client_id_ = TF_CLIENTID_NULL;
}

void CTextService::HandleCommitFromWindow() {
  if (!pipe_) return;
  CommitJob job;
  if (!pipe_->TakeJob(&job)) return;
  CommitResult r;
  r.status = CommitStatus::kFailed;
  r.error = CommitError::kTimeout;
  if (pic_) {
    auto* sess = new CEditSession(pic_, Utf8ToWide(job.text));
    HRESULT sess_hr = E_FAIL;
    HRESULT req_hr =
        pic_->RequestEditSession(client_id_, sess, TF_ES_SYNC | TF_ES_READWRITE, &sess_hr);
    if (SUCCEEDED(req_hr) && SUCCEEDED(sess_hr)) {
      if (sess->outcome() == EditOutcome::kCommitted) {
        r.status = CommitStatus::kCommitted;
        r.error = CommitError::kNone;
      } else if (sess->outcome() == EditOutcome::kNoDocument) {
        r.status = CommitStatus::kRejected;
        r.error = CommitError::kNoDocument;
      } else {
        r.status = CommitStatus::kRejected;
        r.error = CommitError::kRejected;
      }
    }
    sess->Release();
  } else {
    r.status = CommitStatus::kRejected;
    r.error = CommitError::kNoDocument;
  }
  pipe_->CompleteJob(r);
}

}  // namespace openime
