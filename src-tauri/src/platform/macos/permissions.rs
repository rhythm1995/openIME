//! macOS 权限探测：辅助功能（AXIsProcessTrustedWithOptions）+ 麦克风（AVFoundation）。
//!
//! - 辅助功能：调 ApplicationServices 的 `AXIsProcessTrustedWithOptions`，
//!   传入 `kAXTrustedCheckOptionPrompt` 可触发系统弹窗（仅首次）。
//! - 麦克风：AVFoundation `AVCaptureDevice authorizationStatusForMediaType:` /
//!   `requestAccessForMediaType:completionHandler:`，经 objc 运行时裸调用。
//!   注意 1：Info.plist 必须含 NSMicrophoneUsageDescription，否则系统不弹授权框。
//!   注意 2：请求必须在主线程发起（TCC 依赖运行循环弹窗），见 issue_microphone_request。

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use voice_core::permissions::{
    PermissionChecker, PermissionKind, PermissionState, PermissionStatus,
};

extern "C" {
    fn AXIsProcessTrustedWithOptions(options: core_foundation::base::CFTypeRef) -> bool;
}

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *const c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn objc_msgSend();
}

// kAXTrustedCheckOptionPrompt 的键，等于 "AXTrustedCheckOptionPrompt"
const PROMPT_KEY: &str = "AXTrustedCheckOptionPrompt";

pub struct MacPermissionChecker;

impl PermissionChecker for MacPermissionChecker {
    fn check(&self, kind: PermissionKind) -> PermissionStatus {
        let (state, hint) = match kind {
            PermissionKind::Accessibility => {
                // AXIsProcessTrusted 只有 bool：区分不了"从未询问"与"被拒绝"。
                // 未授信时返回 NotDetermined（"未授权"）更贴近事实——重装/重新打包后
                // 旧的授权条目对新二进制失效，系统设置里看起来勾了，这里仍为未授信。
                let granted = is_trusted(false);
                let s = if granted {
                    PermissionState::Granted
                } else {
                    PermissionState::NotDetermined
                };
                (
                    s,
                    "系统设置 → 隐私与安全性 → 辅助功能：若已有 openIME 旧条目，先移除再重新添加 /Applications/openIME.app".to_string(),
                )
            }
            PermissionKind::Microphone => {
                let s = microphone_state();
                (
                    s,
                    "系统设置 → 隐私与安全性 → 麦克风，允许 openIME".to_string(),
                )
            }
        };
        PermissionStatus { kind, state, hint }
    }
}

/// 查询辅助功能权限。`prompt=true` 时触发系统询问弹窗。
pub fn is_trusted(prompt: bool) -> bool {
    if !prompt {
        // SAFETY: 传 NULL 不触发弹窗，纯查询。
        return unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) };
    }
    // 构造 { AXTrustedCheckOptionPrompt: true } 触发弹窗。
    use core_foundation::{
        base::{CFTypeRef, TCFType},
        boolean::CFBoolean,
        dictionary::CFDictionary,
        string::CFString,
    };
    let key = CFString::new(PROMPT_KEY);
    let val = CFBoolean::true_value();
    let dict = CFDictionary::from_CFType_pairs(&[(key, val)]);
    // SAFETY: dict 是合法 CFDictionaryRef，函数只读。
    unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as CFTypeRef) }
}

/// 打开「隐私与安全性」对应面板的系统设置深链。
/// pane: "Privacy_Accessibility" / "Privacy_Microphone" 等。
pub fn open_settings_pane(pane: &str) -> Result<(), String> {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("打开系统设置失败：{e}"))?;
    Ok(())
}

// ──────────────── objc 运行时辅助（最小裸绑定） ────────────────

fn class(name: &str) -> Option<*const c_void> {
    let c = CString::new(name).ok()?;
    let ptr = unsafe { objc_getClass(c.as_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

fn sel(name: &str) -> Option<*const c_void> {
    let c = CString::new(name).ok()?;
    Some(unsafe { sel_registerName(c.as_ptr()) })
}

/// objc_msgSend 按具体签名转码调用（arm64/x86_64 下整数/指针返回安全）。
unsafe fn msg_send_3(
    receiver: *const c_void,
    selector: *const c_void,
    arg: *const c_void,
) -> isize {
    let f: unsafe extern "C" fn(*const c_void, *const c_void, *const c_void) -> isize =
        std::mem::transmute(objc_msgSend as *const () as usize);
    f(receiver, selector, arg)
}

/// 自动释放的 NSString（字面量 UTF8）。
fn ns_string(s: &str) -> Option<*const c_void> {
    let cls = class("NSString")?;
    let sel = sel("stringWithUTF8String:")?;
    let c = CString::new(s).ok()?;
    let f: unsafe extern "C" fn(*const c_void, *const c_void, *const c_char) -> *const c_void =
        unsafe { std::mem::transmute(objc_msgSend as *const () as usize) };
    let ptr = unsafe { f(cls, sel, c.as_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

/// AVMediaTypeAudio == @"soun"
fn audio_media_type() -> Option<*const c_void> {
    ns_string("soun")
}

// ──────────────── 麦克风 ────────────────

/// AVAuthorizationStatus: 0 NotDetermined / 1 Restricted / 2 Denied / 3 Authorized。
pub fn microphone_state() -> PermissionState {
    let Some(cls) = class("AVCaptureDevice") else {
        crate::log_warn!("AVCaptureDevice 类不可用（AVFoundation 未加载？）");
        return PermissionState::NotDetermined;
    };
    let Some(sel) = sel("authorizationStatusForMediaType:") else {
        return PermissionState::NotDetermined;
    };
    let Some(media) = audio_media_type() else {
        return PermissionState::NotDetermined;
    };
    let status = unsafe { msg_send_3(cls, sel, media) };
    match status {
        3 => PermissionState::Granted,
        2 => PermissionState::Denied,
        1 => PermissionState::Restricted,
        _ => PermissionState::NotDetermined,
    }
}

// requestAccessForMediaType:completionHandler: 的 block 载体。
// 全局块（无捕获），回调里只写原子标志。
//
// Block ABI（libclosure Block_private.h）：
//   flags: BLOCK_IS_GLOBAL = 1<<28（必须！否则 Block_copy 会当作栈块
//          去调 copy/dispose helper，直接崩在 _Block_release）。
//   descriptor: reserved 在前、size 在后。
#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: extern "C" fn(*const Block, i8),
    descriptor: &'static BlockDescriptor,
}

extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

static BLOCK_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<Block>(),
};

static GRANTED_FLAG: AtomicBool = AtomicBool::new(false);
static DONE_FLAG: AtomicBool = AtomicBool::new(false);
/// 重入保护：全局块与两个标志位是单例，不能并发请求。
static REQUEST_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// completionHandler:^(BOOL granted) —— macOS arm64/x86_64 上 BOOL 为 i8。
extern "C" fn mic_completion(_block: *const Block, granted: i8) {
    GRANTED_FLAG.store(granted != 0, Ordering::SeqCst);
    DONE_FLAG.store(true, Ordering::SeqCst);
}

/// 块必须是 static：BLOCK_IS_GLOBAL 下 Block_copy 返回原指针，
/// 若放栈上，超时返回后 TCC 再回调就会踩到已失效的栈帧。
struct SyncBlock(Block);
// SAFETY: 块内容初始化后永不变更（运行时只读），跨线程使用安全。
unsafe impl Sync for SyncBlock {}

static MIC_BLOCK: SyncBlock = SyncBlock(Block {
    isa: unsafe { &_NSConcreteGlobalBlock as *const c_void },
    flags: 1 << 28, // BLOCK_IS_GLOBAL
    reserved: 0,
    invoke: mic_completion,
    descriptor: &BLOCK_DESCRIPTOR,
});

/// 发起请求前的预检：结果已知时直接返回（true=已授权 / false=已拒绝或受限），
/// None 表示状态为 NotDetermined、需要真正发起请求触发系统弹窗。
pub fn microphone_preflight() -> Option<bool> {
    match microphone_state() {
        PermissionState::Granted => Some(true),
        PermissionState::Denied | PermissionState::Restricted => Some(false),
        _ => None,
    }
}

/// 发起麦克风授权请求（仅发起，不等待结果）。
///
/// **必须在主线程调用**：TCC 依赖运行循环弹出授权框；在无运行循环的后台线程
/// 上请求不会弹窗且直接被拒（实测）。等待结果请用
/// `microphone_request_finished` / `microphone_request_granted` 轮询。
pub fn issue_microphone_request() -> bool {
    // 已有请求在途（用户连点）：忽略，避免并发踩单例标志位。
    if REQUEST_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        crate::log_debug!("麦克风授权请求已在途，忽略重复发起");
        return false;
    }
    let issued = issue_microphone_request_inner();
    if !issued {
        REQUEST_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
    issued
}

fn issue_microphone_request_inner() -> bool {
    let Some(cls) = class("AVCaptureDevice") else {
        crate::log_warn!("请求麦克风授权失败：AVCaptureDevice 不可用");
        return false;
    };
    let Some(sel) = sel("requestAccessForMediaType:completionHandler:") else {
        return false;
    };
    let Some(media) = audio_media_type() else {
        return false;
    };

    GRANTED_FLAG.store(false, Ordering::SeqCst);
    DONE_FLAG.store(false, Ordering::SeqCst);

    let block_ptr: *const c_void = &MIC_BLOCK.0 as *const Block as *const c_void;

    // SAFETY: cls/sel/media 合法；MIC_BLOCK 为 static，任何时刻回调都有效。
    unsafe {
        let f: unsafe extern "C" fn(*const c_void, *const c_void, *const c_void, *const c_void) =
            std::mem::transmute(objc_msgSend as *const () as usize);
        f(cls, sel, media, block_ptr);
    }
    crate::log_info!("麦克风授权请求已发起（主线程），等待系统弹窗回调");
    true
}

/// 授权请求是否已有回调结果。
pub fn microphone_request_finished() -> bool {
    DONE_FLAG.load(Ordering::SeqCst)
}

/// 授权结果（仅在 finished 为 true 时有意义）。
pub fn microphone_request_granted() -> bool {
    GRANTED_FLAG.load(Ordering::SeqCst)
}

/// 结束一次请求（重置重入锁）。命令侧在拿到结果或超时后调用。
pub fn clear_microphone_request() {
    REQUEST_IN_FLIGHT.store(false, Ordering::SeqCst);
}
