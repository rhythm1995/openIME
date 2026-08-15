// DLL 入口与 COM 导出。
// NFR-11.1：DllMain 只做 DisableThreadLibraryCalls + 标记；零网络、零文件写。
#include <windows.h>

#include <new>

#include "guids.h"

// class_factory.cpp 提供。
extern "C" HRESULT WINAPI OpenImeCreateClassFactory(REFCLSID rclsid, REFIID riid,
                                                    void** ppv);
// registry.cpp 提供。
extern "C" HRESULT WINAPI OpenImeRegisterServer();
extern "C" HRESULT WINAPI OpenImeUnregisterServer();

static HMODULE g_module = nullptr;

// registry.cpp 用：取 DLL 自身模块句柄（注册时写本 DLL 的绝对路径）。
extern "C" HMODULE WINAPI OpenImeTsfModule() { return g_module; }

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID) {
  if (reason == DLL_PROCESS_ATTACH) {
    g_module = module;
    DisableThreadLibraryCalls(module);
  }
  return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv) {
  return OpenImeCreateClassFactory(rclsid, riid, ppv);
}

// TSF TIP 常驻进程内（激活/去激活频繁），不支持卸载。
STDAPI DllCanUnloadNow() { return S_FALSE; }

// 只写 HKCU（设计 FR-11.2：per-user，无 UAC；禁止 HKLM）。
STDAPI DllRegisterServer() { return OpenImeRegisterServer(); }
STDAPI DllUnregisterServer() { return OpenImeUnregisterServer(); }
