#include "edit_session.h"

namespace openime {

// 设计 L770-779 伪代码的落地：
//   GetSelection → SetText → 光标折叠到末尾；无选区 → GetStart 起始插入；
//   无 focus document → no_document（宿主回退 R7）。
STDMETHODIMP CEditSession::DoEditSession(TfEditCookie ec) {
  outcome_ = EditOutcome::kRejected;

  TF_SELECTION sel{};
  ULONG fetched = 0;
  ITfRange* range = nullptr;
  if (SUCCEEDED(pic_->GetSelection(ec, TF_DEFAULT_SELECTION, 1, &sel, &fetched)) &&
      fetched > 0 && sel.range) {
    range = sel.range;  // 接管引用（GetSelection 已 AddRef）
  } else {
    // 空选区（无 caret，如空文档）：插入到文档起始。
    if (FAILED(pic_->GetStart(ec, &range)) || !range) {
      outcome_ = EditOutcome::kNoDocument;
      return S_OK;  // 会话本身成功，结果语义在 outcome_
    }
  }

  HRESULT hr = range->SetText(ec, 0, text_.c_str(), static_cast<LONG>(text_.size()));
  if (SUCCEEDED(hr)) {
    // 光标折叠到插入内容末尾（与 IME 上屏行为一致）。
    if (SUCCEEDED(range->Collapse(ec, TF_ANCHOR_END))) {
      TF_SELECTION after{};
      after.style.ase = TF_AE_NONE;
      after.range = range;
      pic_->SetSelection(ec, 1, &after);
    }
    outcome_ = EditOutcome::kCommitted;
  }
  range->Release();
  return S_OK;
}

}  // namespace openime
