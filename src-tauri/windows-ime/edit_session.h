// ITfEditSession：在目标应用的焦点文档上把文本 SetText 到当前选区（CommitText）。
// 只在 TIP 激活线程（目标应用 UI 线程）经 RequestEditSession(TF_ES_SYNC) 调用。
#pragma once
#include <msctf.h>
#include <string>

namespace openime {

enum class EditOutcome { kCommitted, kNoDocument, kRejected };

class CEditSession : public ITfEditSession {
 public:
  CEditSession(ITfContext* pic, std::wstring text)
      : pic_(pic), text_(std::move(text)) {
    pic_->AddRef();
  }

  // IUnknown
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_INVALIDARG;
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfEditSession)) {
      *ppv = static_cast<ITfEditSession*>(this);
    } else {
      *ppv = nullptr;
      return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
  }
  STDMETHODIMP_(ULONG) AddRef() override {
    return InterlockedIncrement(&ref_);
  }
  STDMETHODIMP_(ULONG) Release() override {
    LONG r = InterlockedDecrement(&ref_);
    if (r == 0) delete this;
    return static_cast<ULONG>(r);
  }

  // ITfEditSession
  STDMETHODIMP DoEditSession(TfEditCookie ec) override;

  EditOutcome outcome() const { return outcome_; }

 private:
  LONG ref_ = 1;
  ITfContext* pic_;
  std::wstring text_;
  EditOutcome outcome_ = EditOutcome::kRejected;
};

}  // namespace openime
