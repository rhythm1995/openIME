// openIME TSF TIP 标识。与宿主侧 src-tauri/src/windows_ime/protocol.rs 的常量
// 逐字一致（改一处必须同步另一处）：
//   OPENIME_TEXT_SERVICE_CLSID = {3F8A1C2E-9B47-4D61-8E2A-71C0F4D59B13}
//   OPENIME_PROFILE_GUID       = {B6D24E91-0C53-4A8F-9E17-2A5D8C3F1B40}
//   LANGID = 0x0804（zh-CN）
#pragma once
// initguid 必须在最前：让本文件里的 DEFINE_GUID 产生定义（而非 extern 声明），
// 否则 C++ 下引用 CLSID_OpenImeTsfTip 会链接失败/无法按引用传参。
#include <guiddef.h>
#include <initguid.h>

// {3F8A1C2E-9B47-4D61-8E2A-71C0F4D59B13}
DEFINE_GUID(CLSID_OpenImeTsfTip, 0x3f8a1c2e, 0x9b47, 0x4d61, 0x8e, 0x2a, 0x71,
            0xc0, 0xf4, 0xd5, 0x9b, 0x13);

// {B6D24E91-0C53-4A8F-9E17-2A5D8C3F1B40}
DEFINE_GUID(GUID_OpenImeProfile, 0xb6d24e91, 0x0c53, 0x4a8f, 0x9e, 0x17, 0x2a,
            0x5d, 0x8c, 0x3f, 0x1b, 0x40);

constexpr WORD kOpenImeLangId = 0x0804;
constexpr wchar_t kOpenImeTipDesc[] = L"openIME 语音输入";
