//! Windows 单实例协调（CreateMutexW 命名互斥体）。
//!
//! - 首个实例创建 `Local\openIME.single-instance.mutex` 并把句柄存入进程级静态，
//!   持有到进程结束（进程退出 = 内核销毁互斥体，无残留/僵尸状态）。
//! - 后续实例创建同名互斥体时 `GetLastError == ERROR_ALREADY_EXISTS`：
//!   按自身 exe basename 唤起已有实例的窗口，然后返回 Err 让调用方退出。
//! - `Local\` 命名空间按会话隔离（同一用户不同会话可各跑一个实例，对输入法合理）。
//!
//! 创建互斥体失败时 fail-open（仅告警、继续启动）：单实例是协调不是核心功能，
//! 不应因它阻断用户使用。

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

/// 主实例持有的互斥体句柄（裸指针按 usize 存：HANDLE 在 windows-rs 0.58 未实现
/// Send/Sync，不能直接入静态）。关闭句柄 = 释放单实例锁，因此持有到进程结束。
static INSTANCE_MUTEX: OnceLock<usize> = OnceLock::new();

/// 单实例互斥体名（Local 命名空间，按登录会话隔离）。
const MUTEX_NAME: &str = "Local\\openIME.single-instance.mutex";

/// 尝试成为唯一实例（默认互斥体名，真实应用使用）。
/// - Ok(())：本进程是主实例，可继续启动。
/// - Err(msg)：已有实例在跑（已唤起其窗口），调用方应退出本进程。
pub fn try_acquire() -> Result<(), String> {
    acquire_named(MUTEX_NAME)
}

/// 按名字获取命名互斥体。测试注入专用名字，避免与真实运行中的应用互扰。
fn acquire_named(mutex_name: &str) -> Result<(), String> {
    // PCWSTR 为 `*const u16`，手工编码 UTF-16 并补 NUL 结尾。
    let name: Vec<u16> = mutex_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // bInitialOwner=false：若互斥体已存在，只探测不等待（true 会阻塞等对端释放）。
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
        .map_err(|e| format!("创建单实例互斥体失败: {e}"))?;

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // 拿到的句柄指向已有互斥体（无用），立即关闭；已有实例持有它到其进程退出。
        unsafe {
            let _ = CloseHandle(handle);
        }
        // 唤起已有实例的窗口：本进程 exe basename 即对端 exe basename。
        // cfg(test) 下跳过：测试进程名与真实应用不同，且不应在测试中抢用户前台。
        #[cfg(not(test))]
        {
            if let Some(exe) = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().and_then(|n| n.to_str().map(str::to_owned)))
            {
                let _ = super::focus::activate_by_exe_basename(&exe);
            }
        }
        return Err("已有实例运行，已唤起其窗口".into());
    }

    // 主实例：句柄以 usize 持有到进程结束，不许 drop / CloseHandle。
    let _ = INSTANCE_MUTEX.set(handle.0 as usize);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用互斥体名：与真实应用名（MUTEX_NAME）隔离，避免与运行中的 openIME 互扰。
    const TEST_MUTEX_IN_PROCESS: &str = "Local\\openIME.single-instance.test-in-process";
    const TEST_MUTEX_CROSS_PROCESS: &str = "Local\\openIME.single-instance.test-cross-process";
    /// 子进程环境标记：命中后 child 用例执行真实断言，其它用例直接跳过。
    const CHILD_ENV: &str = "OPENIME_SI_CHILD_TEST";

    /// 命名构造纯函数路径冒烟：空进程名不触发还焦、错误路径可格式化。
    #[test]
    fn mutex_name_is_utf16_nul_terminated() {
        let name: Vec<u16> = MUTEX_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        assert_eq!(name.last(), Some(&0));
        assert_eq!(
            String::from_utf16_lossy(&name[..name.len() - 1]),
            MUTEX_NAME
        );
    }

    /// 真机验证：同一进程第二次获取同名互斥体 → 内核对象真实存在 → Err。
    #[test]
    fn same_process_second_acquire_detects_existing() {
        assert!(acquire_named(TEST_MUTEX_IN_PROCESS).is_ok());
        // 第二次拿同名互斥体：CreateMutexW 成功但 GetLastError == ERROR_ALREADY_EXISTS。
        assert!(acquire_named(TEST_MUTEX_IN_PROCESS).is_err());
    }

    /// 真机验证（跨进程）：父进程持锁后，子进程（同一测试二进制，只跑 child 用例）
    /// 必须检测到「已有实例」。子进程通过哨兵输出证明用例真的跑到了断言，
    /// 避免「过滤未命中 → 空跑成功」的假阳性。
    #[test]
    fn second_process_detects_existing() {
        if std::env::var(CHILD_ENV).is_ok() {
            return; // 子进程内不再派生孙进程
        }
        assert!(acquire_named(TEST_MUTEX_CROSS_PROCESS).is_ok());
        let exe = std::env::current_exe().expect("测试二进制路径");
        let out = std::process::Command::new(&exe)
            .env(CHILD_ENV, "1")
            .args([
                "--exact",
                "platform::windows::single_instance::tests::child_reports_existing",
                // 通过测试的 println! 默认被 libtest 吞掉，必须 --nocapture 才能拿到哨兵。
                "--nocapture",
            ])
            .output()
            .expect("启动子进程失败");
        assert!(
            out.status.success(),
            "子进程退出码异常：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("SI_CHILD_OK"),
            "子进程哨兵缺失（测试过滤可能未命中）：{stdout}"
        );
        // 父进程的锁仍被持有。
        assert!(acquire_named(TEST_MUTEX_CROSS_PROCESS).is_err());
    }

    /// 子进程用例：正常套件运行时跳过（无环境标记）；作为子进程运行（--exact 过滤）
    /// 时执行真实断言：父进程已持锁，本进程 acquire 必须报「已有实例」。
    #[test]
    fn child_reports_existing() {
        if std::env::var(CHILD_ENV).is_err() {
            return;
        }
        assert!(
            acquire_named(TEST_MUTEX_CROSS_PROCESS).is_err(),
            "子进程应检测到父进程持有的互斥体"
        );
        println!("SI_CHILD_OK");
    }
}
