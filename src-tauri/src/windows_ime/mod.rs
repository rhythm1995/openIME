//! R11：Windows TSF 集成。
//!
//! 纯协议 / 决策函数（`protocol`、`profile`）跨平台可单测（NFR-11.2）；
//! `install`：TIP DLL 安装探测 / 自注册 / 系统收录验证；
//! `ipc` / `session`：命名管道 client 与会话控制器。

// R11 WIP：TSF 接线（原生上屏路径）尚未完成，协议/会话层先行实现、暂无调用方；
// 接线完成后移除本豁免。
#![allow(dead_code)]

pub mod install;
pub mod ipc;
pub mod profile;
pub mod protocol;
pub mod session;

/// R11（FR-11.13）：恢复系统默认输入法（设置页「恢复系统输入法」按钮）。
/// openIME 卡住成为当前输入法时的兜底：激活微软拼音 profile（系统默认）。
#[cfg(target_os = "windows")]
pub fn restore_to_system_default() -> Result<(), String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::TextServices::{ITfInputProcessorProfiles, TF_INPUTPROCESSORPROFILE};

    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let uninit = hr.0 == 0;
    let result = (|| -> Result<(), String> {
        unsafe {
            let profiles_res: windows::core::Result<ITfInputProcessorProfiles> = CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            );
            let profiles = profiles_res.map_err(|e| e.to_string())?;
            // 快照当前 → 若是 openIME 则切回微软拼音（系统默认），否则不动。
            let mgr_res: windows::core::Result<
                windows::Win32::UI::TextServices::ITfInputProcessorProfileMgr,
            > = CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            );
            let mut active = TF_INPUTPROCESSORPROFILE::default();
            if let Ok(mgr) = mgr_res {
                if mgr
                    .GetActiveProfile(
                        &windows::Win32::UI::TextServices::GUID_TFCAT_TIP_KEYBOARD,
                        &mut active,
                    )
                    .is_ok()
                {
                    let ours = session::active_profile_is_openime(&active);
                    if !ours {
                        return Ok(()); // 当前不是 openIME，无需恢复。
                    }
                }
            }
            // 微软拼音（系统默认）：{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E} /
            // profile {FA550B04-5AD7-411F-A5AC-CA038EC515D7}，zh-CN。
            let ms_clsid = windows::core::GUID::from_values(
                0x81d4e9c9,
                0x1d3b,
                0x41bc,
                [0x9e, 0x6c, 0x4b, 0x40, 0xbf, 0x79, 0xe3, 0x5e],
            );
            let ms_profile = windows::core::GUID::from_values(
                0xfa550b04,
                0x5ad7,
                0x411f,
                [0xa5, 0xac, 0xca, 0x03, 0x8e, 0xc5, 0x15, 0xd7],
            );
            profiles
                .ActivateLanguageProfile(&ms_clsid, 0x0804, &ms_profile)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if uninit {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(not(target_os = "windows"))]
pub fn restore_to_system_default() -> Result<(), String> {
    Ok(())
}
