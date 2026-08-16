//! R11：WindowsImeSessionController（阶段 B 宿主侧 FFI）。
//!
//! 一次上屏会话 = prepare → submit → restore（FR-11.4）：
//! 1. prepare：快照当前输入法 → Enable + `ActivateProfile(FORSESSION|…)`（仅会话提示）→
//!    对前台 HWND `WM_INPUTLANGCHANGEREQUEST` → 800ms 内连上目标管道并读到 clientReady
//!    （**clientReady 才是激活成功的唯一标准**，KD-10）。
//! 2. submit：SubmitText → 等匹配 sessionId 的 SubmitResult（≤64KiB）。
//! 3. restore：按 `profile::restore_decision` 还原（幂等；Drop 兜底）。

#[cfg(target_os = "windows")]
use super::install::{detect_status, ImeInstallStatus};
#[cfg(target_os = "windows")]
use super::ipc::connect_and_wait_ready;
use super::ipc::{submit_text_frame, IpcClient};
use super::profile::{restore_decision, ImeProfileSnapshot, ProfileRestoreDecision};
use super::protocol::{ImeErrorCode, ImeSubmitStatus, MAX_TEXT_BYTES};

/// prepare 失败分类（映射到回退语义与日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    NotInstalled,
    NoForeground,
    UnsupportedMachine(u16),
    /// 激活提示发出但 800ms 内未见目标 clientReady（含 server pid 不符）。
    NoClientReady(String),
}

/// 已就绪会话：持有管道连接与快照。Drop 时若未显式 restore 则兜底还原。
pub struct PreparedWindowsImeSession {
    saved: Option<ImeProfileSnapshot>,
    client: Option<IpcClient>,
    /// 前台 HWND（restore 的 WM_INPUTLANGCHANGEREQUEST 目标）。usize 存（HWND 非 Send）。
    fg_hwnd: usize,
    /// 激活是否成功（client_ready）→ restore_decision 的 openime_is_current 入参。
    activated: bool,
    restored: std::cell::Cell<bool>,
}

/// 门控（设计 L801）：`tsf_enabled && Installed && ≤64KiB && AMD64`。
/// 纯函数，insert_ex 与单测共用。
pub fn tsf_gate(
    tsf_enabled: bool,
    text_len: usize,
    machine: u16,
    installed: bool,
) -> Result<(), ImeErrorCode> {
    if !tsf_enabled || !installed {
        // 配置关闭 / 未安装：不是「TSF 失败」，调用方直接走原有插入分支。
        return Err(ImeErrorCode::Rejected);
    }
    if text_len > MAX_TEXT_BYTES {
        return Err(ImeErrorCode::TooLarge);
    }
    if !super::protocol::tsf_supported_for_machine(Some(machine)) {
        return Err(ImeErrorCode::Rejected);
    }
    Ok(())
}

// ── GUID 工具（windows-core 0.58 的 GUID 无 Display/FromStr）──

#[cfg(target_os = "windows")]
fn guid_to_literal(g: &windows::core::GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

/// "{3F8A1C2E-9B47-...}"（大小写/花括号可选）→ GUID。解析失败 panic：入参全部是内置常量。
#[cfg(target_os = "windows")]
fn guid_from_literal(s: &str) -> windows::core::GUID {
    let hex: String = s
        .trim_matches(|c| c == '{' || c == '}')
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    // 8-4-4-4-12 = 32 个 hex 字符。
    assert_eq!(hex.len(), 32, "GUID 字面量 {s} 不合法");
    let take = |r: std::ops::Range<usize>| u32::from_str_radix(&hex[r], 16).expect("hex");
    let d1 = take(0..8);
    let d2 = take(8..12) as u16;
    let d3 = take(12..16) as u16;
    let mut d4 = [0u8; 8];
    for (i, b) in d4.iter_mut().enumerate() {
        *b = take(16 + i * 2..18 + i * 2) as u8;
    }
    windows::core::GUID::from_values(d1, d2, d3, d4)
}

#[cfg(target_os = "windows")]
pub fn prepare_session(
    app: Option<&tauri::AppHandle>,
) -> Result<PreparedWindowsImeSession, PrepareError> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::TextServices::{
        ITfInputProcessorProfileMgr, ITfInputProcessorProfiles, TF_INPUTPROCESSORPROFILE,
        TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE, TF_IPPMF_ENABLEPROFILE, TF_IPPMF_FORSESSION,
        TF_PROFILETYPE_INPUTPROCESSOR,
    };

    if !matches!(detect_status(app), ImeInstallStatus::Installed { .. }) {
        return Err(PrepareError::NotInstalled);
    }
    let Some(info) = crate::platform::windows::focus::frontmost_process_info() else {
        return Err(PrepareError::NoForeground);
    };
    if !super::protocol::tsf_supported_for_machine(Some(info.machine)) {
        return Err(PrepareError::UnsupportedMachine(info.machine));
    }
    let fg_hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if fg_hwnd.0.is_null() {
        return Err(PrepareError::NoForeground);
    }

    let clsid = guid_from_literal(super::protocol::OPENIME_TEXT_SERVICE_CLSID);
    let profile_guid = guid_from_literal(super::protocol::OPENIME_PROFILE_GUID);
    let lang = super::protocol::OPENIME_TSF_LANG_ID;

    // COM：STA（uia.rs 同款：RPC_E_CHANGED_MODE 视为可用但不反初始化）。
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let uninit = hr.0 == 0; // S_OK
    let result = (|| -> Result<PreparedWindowsImeSession, PrepareError> {
        unsafe {
            let mgr: ITfInputProcessorProfileMgr = match CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            ) {
                Ok(m) => m,
                Err(_) => return Err(PrepareError::NotInstalled),
            };
            let profiles: ITfInputProcessorProfiles = match CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            ) {
                Ok(m) => m,
                Err(_) => return Err(PrepareError::NotInstalled),
            };

            // 1) 快照当前活动 profile（按 Keyboard 类别查）。
            let mut active = TF_INPUTPROCESSORPROFILE::default();
            let saved = mgr
                .GetActiveProfile(
                    &windows::Win32::UI::TextServices::GUID_TFCAT_TIP_KEYBOARD,
                    &mut active,
                )
                .ok()
                .and_then(|_| snapshot_from(&active));

            // 2) Enable + 会话级激活提示（真正成功以 clientReady 为准，KD-10）。
            let _ = profiles.EnableLanguageProfile(&clsid, lang, &profile_guid, true);
            let _activate_hr = mgr.ActivateProfile(
                TF_PROFILETYPE_INPUTPROCESSOR,
                lang,
                &clsid,
                &profile_guid,
                windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                TF_IPPMF_FORSESSION
                    | TF_IPPMF_ENABLEPROFILE
                    | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
            );

            // 3) 通知前台切换输入法 → 目标进程按需加载 TIP 并建管道。
            use windows::Win32::UI::WindowsAndMessaging::{
                PostMessageW, INPUTLANGCHANGE_SYSCHARSET, WM_INPUTLANGCHANGEREQUEST,
            };
            let _ = PostMessageW(
                fg_hwnd,
                WM_INPUTLANGCHANGEREQUEST,
                windows::Win32::Foundation::WPARAM(INPUTLANGCHANGE_SYSCHARSET as usize),
                windows::Win32::Foundation::LPARAM(0),
            );

            // 4) 等 clientReady（800ms）。
            match connect_and_wait_ready(info.pid, info.tid, 800) {
                Ok(c) => Ok(PreparedWindowsImeSession {
                    saved,
                    client: Some(c),
                    fg_hwnd: fg_hwnd.0 as usize,
                    activated: true,
                    restored: std::cell::Cell::new(false),
                }),
                Err(e) => {
                    // 激活失败同样要走 restore（restore_decision：activation_failed=true）。
                    let s = PreparedWindowsImeSession {
                        saved,
                        client: None,
                        fg_hwnd: fg_hwnd.0 as usize,
                        activated: false,
                        restored: std::cell::Cell::new(false),
                    };
                    s.restore_session();
                    Err(PrepareError::NoClientReady(format!("{e:?}")))
                }
            }
        }
    })();
    if uninit {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(not(target_os = "windows"))]
pub fn prepare_session(
    _app: Option<&tauri::AppHandle>,
) -> Result<PreparedWindowsImeSession, PrepareError> {
    Err(PrepareError::NotInstalled)
}

impl PreparedWindowsImeSession {
    /// 提交文本；成功返回 Committed，失败返回 error_code（调用方决定回退）。
    pub fn submit(&mut self, text: &str) -> Result<ImeSubmitStatus, ImeErrorCode> {
        let Some(client) = self.client.as_ref() else {
            return Err(ImeErrorCode::Timeout);
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        if !client.write_message(&submit_text_frame(&session_id, text)) {
            return Err(ImeErrorCode::Protocol);
        }
        // stale 帧忽略：循环读直到 sessionId 匹配或超时（NFR-11.3：单次 800ms 量级）。
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_millis(1200) {
            if let Some(frame) = client.read_message(200) {
                if let Ok(r) = super::ipc::interpret_result(Some(frame), &session_id) {
                    return Ok(r);
                }
                // stale / 不匹配 → 继续读。
            }
        }
        Err(ImeErrorCode::Timeout)
    }

    /// 还原输入法（幂等；Drop 兜底再调一次）。失败仅记日志（FR-11.5）。
    pub fn restore_session(&self) {
        if self.restored.get() {
            return;
        }
        self.restored.set(true);
        let decision = restore_decision(self.saved.as_ref(), self.activated, !self.activated);
        if matches!(decision, ProfileRestoreDecision::RestoreSavedProfile) {
            #[cfg(target_os = "windows")]
            {
                let Some(saved) = self.saved.as_ref() else {
                    return;
                };
                unsafe {
                    use windows::Win32::System::Com::{
                        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                        COINIT_APARTMENTTHREADED,
                    };
                    use windows::Win32::UI::TextServices::ITfInputProcessorProfiles;
                    use windows::Win32::UI::WindowsAndMessaging::{
                        PostMessageW, WM_INPUTLANGCHANGEREQUEST,
                    };
                    let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    let uninit = hr.0 == 0;
                    let profiles_res: windows::core::Result<ITfInputProcessorProfiles> =
                        CoCreateInstance(
                            &windows::Win32::UI::TextServices::CLSID_TF_InputProcessorProfiles,
                            None,
                            CLSCTX_INPROC_SERVER,
                        );
                    if let Ok(profiles) = profiles_res {
                        let hwnd = windows::Win32::Foundation::HWND(self.fg_hwnd as *mut _);
                        match saved {
                            ImeProfileSnapshot::TextService {
                                lang,
                                clsid,
                                profile_guid,
                            } => {
                                // 现代优先：ActivateLanguageProfile；失败仅告警（尽力而为）。
                                let c = guid_from_literal(clsid);
                                let g = guid_from_literal(profile_guid);
                                let _ = profiles.ActivateLanguageProfile(&c, *lang, &g);
                                let _ = PostMessageW(
                                    hwnd,
                                    WM_INPUTLANGCHANGEREQUEST,
                                    windows::Win32::Foundation::WPARAM(0),
                                    windows::Win32::Foundation::LPARAM(0),
                                );
                            }
                            ImeProfileSnapshot::KeyboardLayout { hkl, .. } => {
                                let _ = PostMessageW(
                                    hwnd,
                                    WM_INPUTLANGCHANGEREQUEST,
                                    windows::Win32::Foundation::WPARAM(0),
                                    windows::Win32::Foundation::LPARAM(*hkl as isize),
                                );
                            }
                        }
                    }
                    if uninit {
                        CoUninitialize();
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for PreparedWindowsImeSession {
    fn drop(&mut self) {
        self.restore_session();
    }
}

#[cfg(target_os = "windows")]
fn snapshot_from(
    p: &windows::Win32::UI::TextServices::TF_INPUTPROCESSORPROFILE,
) -> Option<ImeProfileSnapshot> {
    use windows::Win32::UI::TextServices::{
        TF_PROFILETYPE_INPUTPROCESSOR, TF_PROFILETYPE_KEYBOARDLAYOUT,
    };
    if p.dwProfileType == TF_PROFILETYPE_KEYBOARDLAYOUT {
        Some(ImeProfileSnapshot::KeyboardLayout {
            lang: p.langid,
            hkl: p.hkl.0 as u64,
        })
    } else if p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR {
        Some(ImeProfileSnapshot::TextService {
            lang: p.langid,
            clsid: guid_to_literal(&p.clsid),
            profile_guid: guid_to_literal(&p.guidProfile),
        })
    } else {
        None
    }
}

/// 当前激活 profile 是否为 openIME（mod.rs 的恢复按钮用：是 → 切回系统默认）。
#[cfg(target_os = "windows")]
pub fn active_profile_is_openime(
    p: &windows::Win32::UI::TextServices::TF_INPUTPROCESSORPROFILE,
) -> bool {
    guid_to_literal(&p.clsid).eq_ignore_ascii_case(super::protocol::OPENIME_TEXT_SERVICE_CLSID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_rejects_non_amd64_and_oversize() {
        assert!(tsf_gate(true, 10, 0x8664, true).is_ok());
        assert_eq!(
            tsf_gate(true, 10, 0x014c, true),
            Err(ImeErrorCode::Rejected)
        );
        assert_eq!(
            tsf_gate(true, 10, 0xaa64, true),
            Err(ImeErrorCode::Rejected)
        );
        assert_eq!(
            tsf_gate(true, 65537, 0x8664, true),
            Err(ImeErrorCode::TooLarge)
        );
        assert_eq!(
            tsf_gate(false, 10, 0x8664, true),
            Err(ImeErrorCode::Rejected)
        );
        assert_eq!(
            tsf_gate(true, 10, 0x8664, false),
            Err(ImeErrorCode::Rejected)
        );
    }

    /// 真机金丝雀（R11）：双模式断言——
    /// - Installed（管理员 regsvr32 过，系统收录）：prepare → submit → Committed → restore；
    /// - Broken/NotInstalled（Win11 per-user 限制）：prepare 应 NotInstalled（零成本回退保护）。
    ///
    /// 手动运行：聚焦记事本后 `cargo test --lib real_tsf -- --ignored --nocapture`。
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "真机金丝雀：需前台聚焦记事本等 TSF 文本应用后手动运行"]
    fn real_tsf_commit_into_foreground_app() {
        use crate::windows_ime::install::{detect_status, ImeInstallStatus};
        let status = detect_status(None);
        println!("TIP 探测：{status:?}");
        match status {
            ImeInstallStatus::Installed { .. } => {
                let info = crate::platform::windows::focus::frontmost_process_info()
                    .expect("需要前台窗口");
                println!(
                    "前台目标：pid={} tid={} machine={:#x}",
                    info.pid, info.tid, info.machine
                );
                let mut s =
                    prepare_session(None).expect("prepare 应成功（激活 + 800ms clientReady）");
                let r = s.submit("openIME TSF CommitText 上屏测试。");
                s.restore_session();
                println!("submit 结果：{r:?}");
                assert_eq!(r, Ok(ImeSubmitStatus::Committed), "提交应成功");
            }
            _ => {
                // 未收录：门控必须把 prepare 挡在 NotInstalled（不得白等 800ms）。
                assert!(matches!(
                    prepare_session(None),
                    Err(PrepareError::NotInstalled)
                ));
                println!("系统未收录 TIP（per-user 限制）→ 回退保护生效 ✓");
            }
        }
    }

    /// GUID 字面量 ↔ windows GUID 往返（macOS 上无 windows::core::GUID，仅 Windows 跑）。
    #[cfg(target_os = "windows")]
    #[test]
    fn guid_literal_roundtrip() {
        for lit in [
            crate::windows_ime::protocol::OPENIME_TEXT_SERVICE_CLSID,
            crate::windows_ime::protocol::OPENIME_PROFILE_GUID,
        ] {
            let g = guid_from_literal(lit);
            assert_eq!(guid_to_literal(&g), lit.to_ascii_uppercase());
        }
    }

    /// snapshot_from：KeyboardLayout / TextService 两种 profile 的映射。
    #[cfg(target_os = "windows")]
    #[test]
    fn snapshot_maps_both_profile_kinds() {
        use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
        use windows::Win32::UI::TextServices::{
            TF_INPUTPROCESSORPROFILE, TF_PROFILETYPE_INPUTPROCESSOR, TF_PROFILETYPE_KEYBOARDLAYOUT,
        };
        let kb = TF_INPUTPROCESSORPROFILE {
            dwProfileType: TF_PROFILETYPE_KEYBOARDLAYOUT,
            langid: 0x0809,
            hkl: HKL(0x0809 as *mut _),
            ..Default::default()
        };
        assert_eq!(
            snapshot_from(&kb),
            Some(ImeProfileSnapshot::KeyboardLayout {
                lang: 0x0809,
                hkl: 0x809,
            })
        );

        let ts = TF_INPUTPROCESSORPROFILE {
            dwProfileType: TF_PROFILETYPE_INPUTPROCESSOR,
            langid: 0x0804,
            clsid: guid_from_literal(crate::windows_ime::protocol::OPENIME_TEXT_SERVICE_CLSID),
            guidProfile: guid_from_literal(crate::windows_ime::protocol::OPENIME_PROFILE_GUID),
            ..Default::default()
        };
        match snapshot_from(&ts) {
            Some(ImeProfileSnapshot::TextService {
                lang,
                clsid,
                profile_guid,
            }) => {
                assert_eq!(lang, 0x0804);
                assert_eq!(
                    clsid,
                    crate::windows_ime::protocol::OPENIME_TEXT_SERVICE_CLSID
                );
                assert_eq!(
                    profile_guid,
                    crate::windows_ime::protocol::OPENIME_PROFILE_GUID
                );
            }
            other => panic!("应映射为 TextService：{other:?}"),
        }
        // active_profile_is_openime：我们的 CLSID → true；微软拼音 → false。
        assert!(active_profile_is_openime(&ts));
        let ms = TF_INPUTPROCESSORPROFILE {
            clsid: guid_from_literal("{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E}"),
            ..Default::default()
        };
        assert!(!active_profile_is_openime(&ms));
    }
}
