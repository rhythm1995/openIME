// IClassFactory → CTextService。
#include <msctf.h>
#include <windows.h>

#include "guids.h"
#include "text_service.h"

namespace openime {

class CClassFactory : public IClassFactory {
 public:
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_INVALIDARG;
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IClassFactory)) {
      *ppv = static_cast<IClassFactory*>(this);
    } else {
      *ppv = nullptr;
      return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&ref_); }
  STDMETHODIMP_(ULONG) Release() override {
    LONG r = InterlockedDecrement(&ref_);
    if (r == 0) delete this;
    return static_cast<ULONG>(r);
  }
  STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override {
    if (!ppv) return E_INVALIDARG;
    if (outer) return CLASS_E_NOAGGREGATION;
    auto* svc = new (std::nothrow) CTextService();
    if (!svc) return E_OUTOFMEMORY;
    HRESULT hr = svc->QueryInterface(riid, ppv);
    svc->Release();
    return hr;
  }
  STDMETHODIMP LockServer(BOOL) override { return S_OK; }

 private:
  LONG ref_ = 1;
};

}  // namespace openime

// dllmain.cpp 里声明导出时使用。
extern "C" HRESULT WINAPI OpenImeCreateClassFactory(REFCLSID rclsid, REFIID riid,
                                                    void** ppv) {
  using namespace openime;
  if (!ppv) return E_POINTER;
  if (!IsEqualCLSID(rclsid, CLSID_OpenImeTsfTip)) return CLASS_E_CLASSNOTAVAILABLE;
  auto* f = new (std::nothrow) CClassFactory();
  if (!f) return E_OUTOFMEMORY;
  HRESULT hr = f->QueryInterface(riid, ppv);
  f->Release();
  return hr;
}
