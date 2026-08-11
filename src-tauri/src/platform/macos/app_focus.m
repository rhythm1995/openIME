// 前台应用激活 + overlay 无激活显示。
//
// 目标：按 Fn 显示录音 HUD 时，用户当前 input 的光标/焦点不能丢。
// 硬约束：
// 1) 所有 AppKit 调用在主线程（录音结束可能在 tokio worker）。
// 2) 显示 overlay 不得 makeKey、不得激活 openIME。
// 3) 若仍误抢了 key window，立刻把 key + firstResponder 还回去。

#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import <objc/runtime.h>

static NSString *const kOpenIMEBundleId = @"com.openime.desktop";

// 在主线程执行 block。
static void openime_on_main(void (^block)(void)) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

// 返回当前前台 app 的 bundleIdentifier（malloc 字符串，调用方 free）。
const char* openime_frontmost_bundle_id(void) {
    __block const char *out = NULL;
    openime_on_main(^{
        NSRunningApplication *app = [[NSWorkspace sharedWorkspace] frontmostApplication];
        if (app && app.bundleIdentifier) {
            const char *cstr = [app.bundleIdentifier UTF8String];
            if (cstr) out = strdup(cstr);
        }
    });
    return out;
}

// 按 bundleIdentifier 激活 app。成功 1 / 失败 0。
// 注意：只能恢复「前台 app」，不能恢复对方内部的 firstResponder；
// 因此更关键的是显示 overlay 时根本不要抢激活。
int openime_activate_app(const char* bundle_id) {
    if (!bundle_id) return 0;
    __block int result = 0;
    NSString *bid = [NSString stringWithUTF8String:bundle_id];
    openime_on_main(^{
        NSArray *apps = [NSRunningApplication runningApplicationsWithBundleIdentifier:bid];
        for (NSRunningApplication *app in apps) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
            // 尽量把前台还回去；IgnoringOtherApps 在部分系统上更有效。
            BOOL ok = [app activateWithOptions:NSApplicationActivateIgnoringOtherApps];
#pragma clang diagnostic pop
            if (!ok) {
                ok = [app activateWithOptions:NSApplicationActivateAllWindows];
            }
            result = ok ? 1 : 0;
            break;
        }
    });
    return result;
}

// 配置 HUD 外观（调用方已保证主线程，或本函数自己切主线程）。
static void openime_configure_hud(NSWindow *w) {
    [w setIgnoresMouseEvents:YES];
    [w setHidesOnDeactivate:NO];
    [w setReleasedWhenClosed:NO];
    [w setAnimationBehavior:NSWindowAnimationBehaviorNone];
    // 注意：不要加 Transient——openIME 是 Accessory，显示 HUD 后会把前台
    // 还给用户 App，Transient 窗在本进程失活时会被系统 orderOut，表现为「一闪而过」。
    w.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces
        | NSWindowCollectionBehaviorFullScreenAuxiliary
        | NSWindowCollectionBehaviorIgnoresCycle
        | NSWindowCollectionBehaviorStationary;
    // 状态栏级：浮在普通窗口上，但不走 key window 路径。
    [w setLevel:NSStatusWindowLevel];
    // 透明点击穿透已在 ignoresMouseEvents。
    [w setOpaque:NO];
    [w setBackgroundColor:[NSColor clearColor]];
    [w setHasShadow:NO];

    // 若运行时窗口实际是 NSPanel，打开 nonactivating 位。
    if ([w isKindOfClass:[NSPanel class]]) {
        NSPanel *p = (NSPanel *)w;
        NSWindowStyleMask mask = [p styleMask] | NSWindowStyleMaskNonactivatingPanel;
        [p setStyleMask:mask];
        [p setBecomesKeyOnlyIfNeeded:YES];
        [p setFloatingPanel:YES];
        [p setWorksWhenModal:YES];
    }
}

void openime_prepare_overlay_window(void *ns_window) {
    if (!ns_window) return;
    openime_on_main(^{
        openime_configure_hud((__bridge NSWindow *)ns_window);
    });
}

// 显示 overlay：不抢 key / 尽量不激活；若误抢则还原 key+firstResponder。
// x/y：由 Rust 传入的「期望左下角位置」提示；最终以 NSScreen.visibleFrame 校正
// （AppKit 原点在左下，避免和 Tauri 顶原点混用）。
// restore_bundle_id：显示前的前台 app；非 openIME 时会在显示后还焦。
void openime_show_overlay_preserving_focus(void *ns_window, double x, double y,
                                           const char *restore_bundle_id) {
    if (!ns_window) return;
    NSString *restoreBid = restore_bundle_id
        ? [NSString stringWithUTF8String:restore_bundle_id]
        : nil;
    // 避免 unused 警告：x/y 仅作 fallback。
    (void)x;
    (void)y;

    openime_on_main(^{
        NSWindow *w = (__bridge NSWindow *)ns_window;

        // 1) 记住当前 key window + firstResponder（同 app 内输入框靠这个恢复）。
        NSWindow *prevKey = [NSApp keyWindow];
        __strong NSResponder *prevFirst = prevKey ? [prevKey firstResponder] : nil;
        NSRunningApplication *prevFront =
            [[NSWorkspace sharedWorkspace] frontmostApplication];

        openime_configure_hud(w);

        // 2) 定位到主屏可见区域左下角上方（AppKit 坐标，原点左下）。
        NSScreen *screen = [w screen] ?: [NSScreen mainScreen];
        NSRect vis = screen ? [screen visibleFrame] : NSMakeRect(0, 0, 1280, 800);
        NSRect frame = [w frame];
        CGFloat pad = 16.0;
        CGFloat bottomPad = 60.0;
        frame.origin.x = NSMinX(vis) + pad;
        frame.origin.y = NSMinY(vis) + bottomPad;
        [w setFrame:frame display:NO];

        // 3) 显示：Accessory + orderFront 通常不抢激活；
        //    仅在不可见时用 orderFrontRegardless。
        [w orderFront:nil];
        if (![w isVisible]) {
            [w orderFrontRegardless];
        }

        // 4) 若 overlay 变成了 key window，立刻交还。
        if ([w isKeyWindow]) {
            [w resignKeyWindow];
        }
        if (prevKey && prevKey != w) {
            [prevKey makeKeyWindow];
            if (prevFirst) {
                [prevKey makeFirstResponder:prevFirst];
            }
        }

        // 5) 误抢前台 app 时还回（跨 app caret 只能靠「别抢」；activate 是兜底）。
        NSRunningApplication *nowFront =
            [[NSWorkspace sharedWorkspace] frontmostApplication];
        BOOL weAreFront = nowFront &&
            [nowFront.bundleIdentifier isEqualToString:kOpenIMEBundleId];
        BOOL shouldRestoreOther =
            restoreBid.length > 0 &&
            ![restoreBid isEqualToString:kOpenIMEBundleId];

        if (shouldRestoreOther) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
            if (weAreFront) {
                NSArray *apps =
                    [NSRunningApplication runningApplicationsWithBundleIdentifier:restoreBid];
                for (NSRunningApplication *app in apps) {
                    [app activateWithOptions:NSApplicationActivateIgnoringOtherApps];
                    break;
                }
            } else if (prevFront &&
                       ![prevFront.bundleIdentifier isEqualToString:kOpenIMEBundleId]) {
                [prevFront activateWithOptions:NSApplicationActivateIgnoringOtherApps];
            }
#pragma clang diagnostic pop
        }

        // 6) 再确认 overlay 不是 key。
        if ([w isKeyWindow]) {
            [w resignKeyWindow];
        }
    });
}

// 兼容旧符号：无定位、无还焦。
void openime_show_window_without_activating(void *ns_window) {
    openime_show_overlay_preserving_focus(ns_window, 16.0, 60.0, NULL);
}

void openime_hide_window_without_activating(void *ns_window) {
    if (!ns_window) return;
    openime_on_main(^{
        NSWindow *w = (__bridge NSWindow *)ns_window;
        [w orderOut:nil];
    });
}
