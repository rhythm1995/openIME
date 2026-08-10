// 前台应用激活：记录录音开始时的前台 app，录音结束后激活回去。
// 解决 overlay show 导致焦点丢失、enigo 输入到错误窗口的问题。

#import <AppKit/AppKit.h>

// 返回当前前台 app 的 bundleIdentifier（malloc 字符串，调用方负责 free）。
// 失败返回 NULL。
const char* openime_frontmost_bundle_id(void) {
    NSRunningApplication *app = [[NSWorkspace sharedWorkspace] frontmostApplication];
    if (app && app.bundleIdentifier) {
        const char *cstr = [app.bundleIdentifier UTF8String];
        if (cstr) return strdup(cstr);
    }
    return NULL;
}

// 按 bundleIdentifier 激活 app。成功返回 1，失败返回 0。
int openime_activate_app(const char* bundle_id) {
    if (!bundle_id) return 0;
    NSString *bid = [NSString stringWithUTF8String:bundle_id];
    NSArray *apps = [NSRunningApplication runningApplicationsWithBundleIdentifier:bid];
    for (NSRunningApplication *app in apps) {
        // macOS 14+ 起 activateWithOptions 会自动带过其他 app，
        // NSApplicationActivateIgnoringOtherApps 已废弃且无效，故不使用。
        return [app activateWithOptions:NSApplicationActivateAllWindows] ? 1 : 0;
    }
    return 0;
}
