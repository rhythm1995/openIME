//! Windows UI Automation（UIA）选区直读。
//!
//! macOS 通过 AXUIElement 直读前台 app 选中文本；Windows 对应物是 UIA 的
//! `IUIAutomationTextPattern`（TextPattern），本模块补齐该缺口（QA 面板「选中文本」）。
//!
//! 调用链：GetFocusedElement → GetCurrentPatternAs(TextPattern) → GetSelection
//! （IUIAutomationTextRangeArray）→ 逐个 GetText 拼接。
//!
//! 覆盖范围：支持 TextPattern 的应用（记事本、VS Code、浏览器等）。不支持的应用
//! （如部分终端 / 远程桌面）GetCurrentPatternAs 失败 → 返回 None，与 macOS 上
//! 「读不到选区」的表现一致，调用方（QA 面板）已有空选区兜底文案。

use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, IUIAutomationTextRangeArray, UIA_TextPatternId,
};

/// GetText 单段最大字符数（选区远超此值无意义：下游会再截断）。
const MAX_SELECTION_CHARS: i32 = 10_000;

/// 读当前焦点元素的选中文本。任何一步失败返回 None，绝不 panic。
pub fn get_selected_text() -> Option<String> {
    // COM 初始化按线程计：命令线程可能是全新线程（需初始化+反初始化），
    // 也可能是已由其它方初始化过的线程（S_FALSE / RPC_E_CHANGED_MODE，不得反初始化，
    // 否则会拆掉主线程上 WebView2 等其它 COM 使用方的状态）。
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let initialized_now = hr == S_OK; // S_OK 才是本次初始化的，结束时配对反初始化
    let usable = hr.is_ok() || hr == RPC_E_CHANGED_MODE;
    if !usable {
        return None;
    }
    let text = read_selection_via_uia();
    if initialized_now {
        unsafe { CoUninitialize() };
    }
    text
}

/// 假定 COM 已初始化：走 UIA TextPattern 读选区。
fn read_selection_via_uia() -> Option<String> {
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()? };
    let focused: IUIAutomationElement = unsafe { uia.GetFocusedElement().ok()? };
    let pattern: IUIAutomationTextPattern =
        unsafe { focused.GetCurrentPatternAs(UIA_TextPatternId).ok()? };
    let selection: IUIAutomationTextRangeArray = unsafe { pattern.GetSelection().ok()? };
    let count = unsafe { selection.Length().ok()? } as usize;

    let mut texts: Vec<String> = Vec::with_capacity(count);
    for i in 0..count {
        // GetElement 走 from_abi 接管返回引用，drop 时各自 Release。
        let range: IUIAutomationTextRange = unsafe { selection.GetElement(i as i32).ok()? };
        if let Ok(text) = unsafe { range.GetText(MAX_SELECTION_CHARS) } {
            let s = text.to_string();
            if !s.is_empty() {
                texts.push(s);
            }
        }
    }

    let text = texts.concat();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 冒烟测试：不 panic、始终返回 Option。
    /// CI 服务会话可能没有前台焦点元素（返回 None），本机交互会话则可能返回 Some，
    /// 因此只断言「不崩溃」。
    #[test]
    fn get_selected_text_never_panics() {
        let _ = get_selected_text();
        let _ = get_selected_text();
    }

    /// 手动功能冒烟（默认忽略）：先在前台应用（如记事本）选中文字，再运行
    /// `cargo test -p openime uia_reads_focused_selection -- --ignored --nocapture`，
    /// 应打印出选中内容。CI 会话无前台焦点，不参与自动运行。
    #[test]
    #[ignore = "manual smoke: focus an app with selected text first"]
    fn uia_reads_focused_selection() {
        let sel = get_selected_text();
        println!("UIA_SELECTION={sel:?}");
        assert!(
            sel.is_some(),
            "未读到选区（焦点可能不在支持 TextPattern 的应用上）"
        );
    }
}
