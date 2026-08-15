//! R11：TSF TIP 的安装探测 / 自注册（阶段 A 宿主侧）。
//!
//! DLL 位置：dev 构建在 `src-tauri/ime/`（build.rs 产出）；NSIS 安装在
//! `resource_dir()/ime/`。宿主启动时自检自注册（幂等，仅 HKCU），覆盖 dev 直跑、
//! 安装包、应用更新三种场景；安装器的 hooks.nsh 也注册一次（双保险，操作幂等）。

use std::path::PathBuf;

use tauri::Manager;

use super::protocol::OPENIME_TEXT_SERVICE_CLSID;

/// 探测结果（设计 FR-11.11：Installed / NotInstalled / RegistrationBroken）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeInstallStatus {
    Installed { dll: PathBuf },
    NotInstalled,
    RegistrationBroken { reason: String },
}

/// 纯决策：注册表值 × 磁盘事实 → 状态（跨平台可单测）。
///
/// - 无注册值 → NotInstalled；
/// - 注册的路径 == 某个存在的候选 DLL 且 TIP 键在 → Installed；
/// - 有注册值但文件不在 / 路径与候选不符 / TIP 键缺失 → RegistrationBroken。
pub fn classify_ime_status(
    registered: Option<&str>,
    candidates: &[PathBuf],
    file_exists: impl Fn(&std::path::Path) -> bool,
    tip_key_present: bool,
) -> ImeInstallStatus {
    let Some(reg) = registered else {
        return ImeInstallStatus::NotInstalled;
    };
    let reg_path = PathBuf::from(reg);
    let same = |a: &std::path::Path, b: &std::path::Path| -> bool {
        // 大小写不敏感比较（Windows 路径语义），canonicalize 交给调用方的 exists 探测。
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    };
    let matches_candidate = candidates
        .iter()
        .any(|c| same(&reg_path, c) && file_exists(c));
    if !matches_candidate {
        // 注册路径指向的文件还在（可能是旧位置残留）→ 也算可用？不：
        // 设计要求路径与当前候选一致，防止劫持/陈旧（L822）。
        return ImeInstallStatus::RegistrationBroken {
            reason: format!("注册路径与当前 DLL 不符：{reg}"),
        };
    }
    if !tip_key_present {
        return ImeInstallStatus::RegistrationBroken {
            reason: "TSF TIP LanguageProfile 键缺失".into(),
        };
    }
    ImeInstallStatus::Installed {
        dll: reg_path,
    }
}

/// DLL 候选路径：dev（manifest/ime）→ 安装包（resource_dir/ime）→ exe 同级 ime/ → exe 同级。
pub fn dll_candidate_paths(app: Option<&tauri::AppHandle>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(app) = app {
        if let Ok(rd) = app.path().resource_dir() {
            v.push(rd.join("ime").join("OpenImeTsf.dll"));
        }
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        // dev 直跑：build.rs 产物目录（编译期常量，发布构建无害——目录不存在即被跳过）。
        v.push(PathBuf::from(manifest).join("ime").join("OpenImeTsf.dll"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("ime").join("OpenImeTsf.dll"));
            v.push(dir.join("OpenImeTsf.dll"));
        }
    }
    v
}

/// 供 `classify_ime_status` 使用的注册表读（Windows FFI，其它平台恒 None）。
#[cfg(target_os = "windows")]
fn registered_inproc_path() -> Option<String> {
    let key = format!(
        r"Software\Classes\CLSID\{}\InprocServer32",
        OPENIME_TEXT_SERVICE_CLSID
    );
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(key)
        .ok()?
        .get_value::<String, _>("")
        .ok()
}

#[cfg(not(target_os = "windows"))]
fn registered_inproc_path() -> Option<String> {
    None
}

/// TSF TIP LanguageProfile 键是否存在（HKCU\Software\Microsoft\CTF\TIP\{clsid}）。
#[cfg(target_os = "windows")]
fn tip_key_present() -> bool {
    let key = format!(
        r"Software\Microsoft\CTF\TIP\{}\LanguageProfile",
        OPENIME_TEXT_SERVICE_CLSID
    );
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(key)
        .is_ok()
}

#[cfg(not(target_os = "windows"))]
fn tip_key_present() -> bool {
    false
}

/// 探测当前安装状态。
/// Windows 侧额外做「枚举验证」：注册键在但 msctf 不收录（per-user 限制）→
/// RegistrationBroken（避免每次插入白等 800ms 激活超时）。
pub fn detect_status(app: Option<&tauri::AppHandle>) -> ImeInstallStatus {
    let base = classify_ime_status(
        registered_inproc_path().as_deref(),
        &dll_candidate_paths(app),
        |p| p.is_file(),
        tip_key_present(),
    );
    #[cfg(target_os = "windows")]
    {
        if matches!(base, ImeInstallStatus::Installed { .. }) && !system_lists_tip() {
            return ImeInstallStatus::RegistrationBroken {
                reason: "注册键在但系统未收录（Win11 per-user TIP 限制；需管理员 \
                         regsvr32 写 HKLM）"
                    .into(),
            };
        }
    }
    base
}

/// 自注册：DLL 存在但状态非 Installed 时，LoadLibrary + DllRegisterServer（幂等）。
/// 返回注册后的状态。DLL 缺失（构建跳过）→ NotInstalled，静默走 R7。
#[cfg(target_os = "windows")]
pub fn ensure_registered(app: Option<&tauri::AppHandle>) -> ImeInstallStatus {
    use windows::core::PCSTR;
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    let candidates = dll_candidate_paths(app);
    let status = detect_status(app);
    if matches!(status, ImeInstallStatus::Installed { .. }) {
        return status;
    }
    let Some(dll) = candidates.iter().find(|p| p.is_file()) else {
        return ImeInstallStatus::NotInstalled;
    };
    let path_bytes = match dll.to_str() {
        Some(s) => format!("{s}\0"),
        None => return status,
    };
    unsafe {
        let Ok(h) = LoadLibraryA(PCSTR(path_bytes.as_ptr())) else {
            return ImeInstallStatus::RegistrationBroken {
                reason: format!("DLL 加载失败：{}", dll.display()),
            };
        };
        let reg = GetProcAddress(h, PCSTR(b"DllRegisterServer\0".as_ptr()));
        let hr = if let Some(f) = reg {
            let f: extern "system" fn() -> i32 = std::mem::transmute(f);
            f()
        } else {
            -1
        };
        let _ = FreeLibrary(HMODULE(h.0));
        if hr != 0 {
            return ImeInstallStatus::RegistrationBroken {
                reason: format!("DllRegisterServer hr=0x{hr:08X}"),
            };
        }
    }
    crate::log_info!("TSF TIP 自注册完成：{}", dll.display());
    detect_status(app)
}

/// 系统（msctf）是否真的收录了我们的 TIP：EnumProfiles 按我们的 CLSID 匹配。
/// Win11 实测：HKCU 注册键齐全时枚举仍可能无视（只认 HKLM）——这是
/// Installed 与 RegistrationBroken 的分界，避免每次插入白等 800ms 激活超时。
#[cfg(target_os = "windows")]
pub fn system_lists_tip() -> bool {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::TextServices::TF_INPUTPROCESSORPROFILE;

    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let uninit = hr.0 == 0;
    let mut listed = false;
    if hr.0 == 0 || hr.0 == -2147417850 {
        // S_OK 或 RPC_E_CHANGED_MODE
        unsafe {
            let mgr_res: windows::core::Result<
                windows::Win32::UI::TextServices::ITfInputProcessorProfileMgr,
            > = CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            );
            if let Ok(mgr) = mgr_res {
                let Ok(iter) = mgr.EnumProfiles(0) else {
                    if uninit {
                        CoUninitialize();
                    }
                    return false;
                };
                let ours = guid_bytes(super::protocol::OPENIME_TEXT_SERVICE_CLSID);
                let mut items = [TF_INPUTPROCESSORPROFILE::default(); 32];
                let mut fetched = 0u32;
                if iter.Next(&mut items, &mut fetched).is_ok() {
                    for item in &items[..fetched as usize] {
                        if item.clsid.data1 == u32::from_be_bytes([
                            ours[0], ours[1], ours[2], ours[3],
                        ]) {
                            listed = true;
                            break;
                        }
                    }
                }
            }
        }
    }
    if uninit {
        unsafe { CoUninitialize() };
    }
    listed
}

/// "{...}" GUID 字面量 → 16 字节（data1 大端 u32 的低 4 字节比较用）。
#[cfg(target_os = "windows")]
fn guid_bytes(s: &str) -> [u8; 16] {
    let hex: Vec<u8> = s
        .trim_matches(|c| c == '{' || c == '}')
        .bytes()
        .filter(|b| b.is_ascii_hexdigit())
        .collect();
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        let hi = (hex[i * 2] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (hex[i * 2 + 1] as char).to_digit(16).unwrap_or(0) as u8;
        *b = hi << 4 | lo;
    }
    out
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_registered(app: Option<&tauri::AppHandle>) -> ImeInstallStatus {
    detect_status(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn no_registration_means_not_installed() {
        assert_eq!(
            classify_ime_status(None, &[], |_| false, false),
            ImeInstallStatus::NotInstalled
        );
    }

    #[test]
    fn registered_and_present_with_tip_key_is_installed() {
        let candidates = [p("C:/app/ime/OpenImeTsf.dll")];
        assert_eq!(
            classify_ime_status(
                Some("C:/app/ime/OpenImeTsf.dll"),
                &candidates,
                |_| true,
                true
            ),
            ImeInstallStatus::Installed {
                dll: p("C:/app/ime/OpenImeTsf.dll")
            }
        );
        // 路径大小写不敏感。
        assert_eq!(
            classify_ime_status(
                Some("c:/APP/IME/OPENIMETSF.DLL"),
                &candidates,
                |_| true,
                true
            ),
            ImeInstallStatus::Installed {
                dll: p("c:/APP/IME/OPENIMETSF.DLL")
            }
        );
    }

    #[test]
    fn registered_but_file_missing_or_mismatch_is_broken() {
        let candidates = [p("C:/new/ime/OpenImeTsf.dll")];
        // 文件不在。
        assert!(matches!(
            classify_ime_status(Some("C:/new/ime/OpenImeTsf.dll"), &candidates, |_| false, true),
            ImeInstallStatus::RegistrationBroken { .. }
        ));
        // 注册的是旧路径（与当前候选不符）→ Broken（防陈旧/劫持，设计 L822）。
        assert!(matches!(
            classify_ime_status(Some("C:/old/OpenImeTsf.dll"), &candidates, |_| true, true),
            ImeInstallStatus::RegistrationBroken { .. }
        ));
    }

    #[test]
    fn installed_but_tip_key_missing_is_broken() {
        let candidates = [p("C:/app/ime/OpenImeTsf.dll")];
        assert!(matches!(
            classify_ime_status(Some("C:/app/ime/OpenImeTsf.dll"), &candidates, |_| true, false),
            ImeInstallStatus::RegistrationBroken { .. }
        ));
    }
}
