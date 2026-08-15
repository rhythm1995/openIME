// TIP 注册（阶段 A 终版）。
//
// Win11 实测结论（2026-08-15）：输入法枚举（EnumProfiles）与激活（ActivateProfile）
// 只认 HKLM\SOFTWARE\Microsoft\CTF 下的 TIP 注册；per-user（HKCU）键写齐也不被收录。
// msctf 的 ITfInputProcessorProfiles::AddLanguageProfile 恒写 HKCU（提升进程亦然），
// 故 HKLM 路径全部手写注册表（照抄系统输入法的键形态）。
// 策略：管理员 regsvr32 → 写 HKLM（可激活）；普通权限 → 写 HKCU
//（宿主探测 RegistrationBroken → 零成本回退 R7 插入）。
#include <msctf.h>
#include <objbase.h>
#include <olectl.h>  // SELFREG_E_CLASS
#include <windows.h>

#include <string>

#include "guids.h"

// dllmain.cpp 提供：DLL 自身模块句柄。
extern "C" HMODULE WINAPI OpenImeTsfModule();

namespace openime {
namespace {

// 键名里的 CLSID/Profile GUID 字面量（大写带花括号，与系统输入法一致）。
std::wstring ClsidLiteral() {
  wchar_t b[64];
  swprintf_s(b, L"{%08lX-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}",
             CLSID_OpenImeTsfTip.Data1, CLSID_OpenImeTsfTip.Data2,
             CLSID_OpenImeTsfTip.Data3, CLSID_OpenImeTsfTip.Data4[0],
             CLSID_OpenImeTsfTip.Data4[1], CLSID_OpenImeTsfTip.Data4[2],
             CLSID_OpenImeTsfTip.Data4[3], CLSID_OpenImeTsfTip.Data4[4],
             CLSID_OpenImeTsfTip.Data4[5], CLSID_OpenImeTsfTip.Data4[6],
             CLSID_OpenImeTsfTip.Data4[7]);
  return b;
}

std::wstring ProfileLiteral() {
  wchar_t b[64];
  swprintf_s(b, L"{%08lX-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}",
             GUID_OpenImeProfile.Data1, GUID_OpenImeProfile.Data2,
             GUID_OpenImeProfile.Data3, GUID_OpenImeProfile.Data4[0],
             GUID_OpenImeProfile.Data4[1], GUID_OpenImeProfile.Data4[2],
             GUID_OpenImeProfile.Data4[3], GUID_OpenImeProfile.Data4[4],
             GUID_OpenImeProfile.Data4[5], GUID_OpenImeProfile.Data4[6],
             GUID_OpenImeProfile.Data4[7]);
  return b;
}

// COM InprocServer32（root=HKLM 时写 HKLM\SOFTWARE\Classes，否则 HKCU\Software\Classes）。
bool WriteComKeys(HKEY root, const std::wstring& dll_path) {
  const std::wstring clsid = ClsidLiteral();
  HKEY h = nullptr;
  if (RegCreateKeyExW(root, (L"SOFTWARE\\Classes\\CLSID\\" + clsid).c_str(), 0,
                      nullptr, 0, KEY_SET_VALUE, nullptr, &h,
                      nullptr) != ERROR_SUCCESS) {
    return false;
  }
  const wchar_t friendly[] = L"openIME TSF Text Service";
  RegSetValueExW(h, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(friendly),
                 sizeof(friendly));
  RegCloseKey(h);
  if (RegCreateKeyExW(root,
                      (L"SOFTWARE\\Classes\\CLSID\\" + clsid +
                       L"\\InprocServer32")
                          .c_str(),
                      0, nullptr, 0, KEY_SET_VALUE, nullptr, &h,
                      nullptr) != ERROR_SUCCESS) {
    return false;
  }
  RegSetValueExW(h, nullptr, 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(dll_path.c_str()),
                 static_cast<DWORD>((dll_path.size() + 1) * sizeof(wchar_t)));
  const wchar_t apt[] = L"Apartment";
  RegSetValueExW(h, L"ThreadingModel", 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(apt), sizeof(apt));
  RegCloseKey(h);
  return true;
}

// TIP 主键下的 LanguageProfile（Description + Enable=1；照抄系统输入法形态）。
void WriteLanguageProfileKeys(HKEY root) {
  wchar_t path[220];
  swprintf_s(path,
             L"SOFTWARE\\Microsoft\\CTF\\TIP\\%s\\LanguageProfile\\0x%08X\\%s",
             ClsidLiteral().c_str(), kOpenImeLangId, ProfileLiteral().c_str());
  HKEY h = nullptr;
  if (RegCreateKeyExW(root, path, 0, nullptr, 0, KEY_SET_VALUE, nullptr, &h,
                      nullptr) != ERROR_SUCCESS) {
    return;
  }
  RegSetValueExW(h, L"Description", 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(kOpenImeTipDesc),
                 sizeof(kOpenImeTipDesc));
  const DWORD one = 1;
  RegSetValueExW(h, L"Enable", 0, REG_DWORD,
                 reinterpret_cast<const BYTE*>(&one), sizeof(one));
  RegCloseKey(h);
}

// TIP 类别：TIP\{clsid}\Category\Category\{catid}\{clsid}（Keyboard 是枚举硬前提）。
void WriteTipCategoryKeys(HKEY root) {
  const wchar_t* cats[] = {
      L"{34745C63-B2F0-4784-8B67-5E12C8701A31}",  // GUID_TFCAT_TIP_KEYBOARD
      L"{13A016DF-560B-46CD-947A-4C3AF1E0E35D}",  // TIPCAP_IMMERSIVESUPPORT
      L"{25504FB4-7BAB-4BC1-9C69-CF81890F0EF5}",  // TIPCAP_SYSTRAYSUPPORT
  };
  for (const wchar_t* cat : cats) {
    wchar_t path[260];
    swprintf_s(path, L"SOFTWARE\\Microsoft\\CTF\\TIP\\%s\\Category\\Category\\%s\\%s",
               ClsidLiteral().c_str(), cat, ClsidLiteral().c_str());
    HKEY h = nullptr;
    if (RegCreateKeyExW(root, path, 0, nullptr, 0, 0, nullptr, &h,
                        nullptr) == ERROR_SUCCESS) {
      RegCloseKey(h);
    }
  }
}

// 输入法列表装配项：SortOrder\AssemblyItem\0x00000804\{Keyboard类别}\{序号}
//（Win10/11 的 InputSwitch/枚举实际读取；语言键是 0x%08X 八位十六进制）。
bool WriteSortOrderAssembly(HKEY root) {
  wchar_t base[200];
  swprintf_s(base,
             L"SOFTWARE\\Microsoft\\CTF\\SortOrder\\AssemblyItem\\0x%08X\\"
             L"{34745C63-B2F0-4784-8B67-5E12C8701A31}",
             kOpenImeLangId);
  const std::wstring clsid = ClsidLiteral();
  const std::wstring profile = ProfileLiteral();
  HKEY h = nullptr;
  if (RegCreateKeyExW(root, base, 0, nullptr, 0, KEY_READ | KEY_WRITE, nullptr,
                      &h, nullptr) != ERROR_SUCCESS) {
    return false;
  }
  wchar_t self_item[16] = L"";
  int max_idx = -1;
  wchar_t name[64];
  for (DWORD i = 0;; ++i) {
    DWORD name_len = 64;
    if (RegEnumKeyExW(h, i, name, &name_len, nullptr, nullptr, nullptr,
                      nullptr) != ERROR_SUCCESS) {
      break;
    }
    int idx = _wtoi(name);
    if (idx > max_idx) max_idx = idx;
    HKEY sub = nullptr;
    if (RegOpenKeyExW(h, name, 0, KEY_QUERY_VALUE, &sub) == ERROR_SUCCESS) {
      wchar_t v[80] = L"";
      DWORD sz = sizeof(v);
      if (RegQueryValueExW(sub, L"CLSID", nullptr, nullptr,
                           reinterpret_cast<BYTE*>(v), &sz) == ERROR_SUCCESS &&
          _wcsicmp(v, clsid.c_str()) == 0) {
        wcscpy_s(self_item, name);
      }
      RegCloseKey(sub);
    }
  }
  if (self_item[0] == L'\0') {
    swprintf_s(self_item, L"%06d", max_idx + 1);
  }
  wchar_t item_path[256];
  swprintf_s(item_path, L"%s\\%s", base, self_item);
  HKEY item = nullptr;
  if (RegCreateKeyExW(root, item_path, 0, nullptr, 0, KEY_SET_VALUE, nullptr,
                      &item, nullptr) != ERROR_SUCCESS) {
    RegCloseKey(h);
    return false;
  }
  RegSetValueExW(item, L"CLSID", 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(clsid.c_str()),
                 static_cast<DWORD>((clsid.size() + 1) * sizeof(wchar_t)));
  RegSetValueExW(item, L"Profile", 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(profile.c_str()),
                 static_cast<DWORD>((profile.size() + 1) * sizeof(wchar_t)));
  const DWORD zero = 0;
  RegSetValueExW(item, L"KeyboardLayout", 0, REG_DWORD,
                 reinterpret_cast<const BYTE*>(&zero), sizeof(zero));
  RegCloseKey(item);
  RegCloseKey(h);
  return true;
}

// 提升检测：能创建 HKLM\SOFTWARE\Microsoft\CTF 下的键 = 管理员。
HKEY PickRegistrationRoot() {
  HKEY probe = nullptr;
  if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\CTF", 0, nullptr,
                      0, KEY_QUERY_VALUE, nullptr, &probe, nullptr) ==
      ERROR_SUCCESS) {
    RegCloseKey(probe);
    return HKEY_LOCAL_MACHINE;
  }
  return HKEY_CURRENT_USER;
}

}  // namespace

bool RegisterTsfProfile(const std::wstring& dll_path) {
  HKEY root = PickRegistrationRoot();
  if (!WriteComKeys(root, dll_path)) return false;
  WriteLanguageProfileKeys(root);
  WriteTipCategoryKeys(root);
  if (!WriteSortOrderAssembly(root)) {
    OutputDebugStringW(L"[openIME TSF] SortOrder 装配项写入失败");
  }

  // msctf COM 侧注册（写 HKCU 的 per-user 存储；对 HKLM 路径是补充，失败不阻塞——
  // 枚举/激活以手写键为准）。
  HRESULT ci = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  const bool uninit = SUCCEEDED(ci);
  if (SUCCEEDED(ci) || ci == RPC_E_CHANGED_MODE) {
    ITfInputProcessorProfiles* profiles = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr,
                                   CLSCTX_INPROC_SERVER,
                                   IID_ITfInputProcessorProfiles,
                                   reinterpret_cast<void**>(&profiles)))) {
      profiles->Register(CLSID_OpenImeTsfTip);
      profiles->AddLanguageProfile(
          CLSID_OpenImeTsfTip, kOpenImeLangId, GUID_OpenImeProfile,
          kOpenImeTipDesc, static_cast<ULONG>(wcslen(kOpenImeTipDesc)), nullptr,
          0, 0);
      profiles->EnableLanguageProfile(CLSID_OpenImeTsfTip, kOpenImeLangId,
                                      GUID_OpenImeProfile, TRUE);
      profiles->Release();
    }
    ITfCategoryMgr* cats = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_CategoryMgr, nullptr,
                                   CLSCTX_INPROC_SERVER, IID_ITfCategoryMgr,
                                   reinterpret_cast<void**>(&cats)))) {
      cats->RegisterCategory(CLSID_OpenImeTsfTip, GUID_TFCAT_TIP_KEYBOARD,
                             CLSID_OpenImeTsfTip);
      cats->RegisterCategory(CLSID_OpenImeTsfTip,
                             GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
                             CLSID_OpenImeTsfTip);
      cats->RegisterCategory(CLSID_OpenImeTsfTip, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
                             CLSID_OpenImeTsfTip);
      cats->Release();
    }
  }
  if (uninit) CoUninitialize();
  return true;
}

bool UnregisterTsfProfile() {
  HKEY root = PickRegistrationRoot();
  const std::wstring clsid = ClsidLiteral();
  // TIP 主键整树删除（含 LanguageProfile / Category）。
  RegDeleteTreeW(root, (L"SOFTWARE\\Microsoft\\CTF\\TIP\\" + clsid).c_str());
  // SortOrder 装配项按 CLSID 值匹配删除。
  wchar_t base[200];
  swprintf_s(base,
             L"SOFTWARE\\Microsoft\\CTF\\SortOrder\\AssemblyItem\\0x%08X\\"
             L"{34745C63-B2F0-4784-8B67-5E12C8701A31}",
             kOpenImeLangId);
  HKEY h = nullptr;
  if (RegOpenKeyExW(root, base, 0, KEY_READ | DELETE, &h) == ERROR_SUCCESS) {
    wchar_t name[64];
    for (DWORD i = 0;; ++i) {
      DWORD name_len = 64;
      if (RegEnumKeyExW(h, i, name, &name_len, nullptr, nullptr, nullptr,
                        nullptr) != ERROR_SUCCESS) {
        break;
      }
      HKEY sub = nullptr;
      if (RegOpenKeyExW(h, name, 0, KEY_QUERY_VALUE, &sub) == ERROR_SUCCESS) {
        wchar_t v[80] = L"";
        DWORD sz = sizeof(v);
        if (RegQueryValueExW(sub, L"CLSID", nullptr, nullptr,
                             reinterpret_cast<BYTE*>(v), &sz) == ERROR_SUCCESS &&
            _wcsicmp(v, clsid.c_str()) == 0) {
          RegCloseKey(sub);
          RegDeleteTreeW(h, name);
          continue;
        }
        RegCloseKey(sub);
      }
    }
    RegCloseKey(h);
  }
  RegDeleteTreeW(root, (L"SOFTWARE\\Classes\\CLSID\\" + clsid).c_str());

  HRESULT ci = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  const bool uninit = SUCCEEDED(ci);
  if (SUCCEEDED(ci) || ci == RPC_E_CHANGED_MODE) {
    ITfInputProcessorProfiles* profiles = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr,
                                   CLSCTX_INPROC_SERVER,
                                   IID_ITfInputProcessorProfiles,
                                   reinterpret_cast<void**>(&profiles)))) {
      profiles->EnableLanguageProfile(CLSID_OpenImeTsfTip, kOpenImeLangId,
                                      GUID_OpenImeProfile, FALSE);
      profiles->Unregister(CLSID_OpenImeTsfTip);
      profiles->Release();
    }
    ITfCategoryMgr* cats = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_CategoryMgr, nullptr,
                                   CLSCTX_INPROC_SERVER, IID_ITfCategoryMgr,
                                   reinterpret_cast<void**>(&cats)))) {
      cats->UnregisterCategory(CLSID_OpenImeTsfTip, GUID_TFCAT_TIP_KEYBOARD,
                               CLSID_OpenImeTsfTip);
      cats->UnregisterCategory(CLSID_OpenImeTsfTip,
                               GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
                               CLSID_OpenImeTsfTip);
      cats->UnregisterCategory(CLSID_OpenImeTsfTip,
                               GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
                               CLSID_OpenImeTsfTip);
      cats->Release();
    }
  }
  if (uninit) CoUninitialize();
  return true;
}

}  // namespace openime

extern "C" HRESULT WINAPI OpenImeRegisterServer() {
  // DLL 自身路径（dllmain.cpp 提供）：被 regsvr32 或宿主 LoadLibrary 调用时都正确。
  HMODULE mod = OpenImeTsfModule();
  if (!mod) return E_FAIL;
  wchar_t path[MAX_PATH * 2];
  DWORD n = GetModuleFileNameW(mod, path, ARRAYSIZE(path));
  if (n == 0 || n == ARRAYSIZE(path)) return E_FAIL;
  return openime::RegisterTsfProfile(path) ? S_OK : SELFREG_E_CLASS;
}

extern "C" HRESULT WINAPI OpenImeUnregisterServer() {
  return openime::UnregisterTsfProfile() ? S_OK : SELFREG_E_CLASS;
}
