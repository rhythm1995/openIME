// Fn 键监听（ObjC）：CGEventTap source 挂到主线程 run loop（common modes）。
// 之前在专用线程上注册 CGEventTap 收不到事件——CGEventTap 的 source 必须
// 挂在主线程的 NSRunLoop 上才能被正确 pump。
// 同时保留 NSEvent global+local monitor 做补充。

#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreFoundation/CoreFoundation.h>

extern void openime_fn_edge(bool pressed);

static bool fn_down = false;

// NSEvent monitor 的 flagsChanged 处理。
static void handle_event(NSEvent *event) {
    NSUInteger flags = event.modifierFlags;
    NSUInteger keycode = event.keyCode;
    // 只处理 Fn 自身（keyCode=63）的 flagsChanged。
    if (keycode != 63) return;
    bool fn_now = (flags & NSEventModifierFlagFunction) != 0;
    if (fn_now != fn_down) {
        fn_down = fn_now;
        openime_fn_edge(fn_now);
    }
}

// CGEventTap 回调（C 函数）。
static CGEventRef cg_callback(CGEventTapProxy proxy, CGEventType type,
                               CGEventRef event, void *refcon) {
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        return event;
    }
    if (type != kCGEventFlagsChanged) return event;
    int64_t keycode = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
    if (keycode != 63) return event;
    CGEventFlags flags = CGEventGetFlags(event);
    bool fn_now = (flags & kCGEventFlagMaskSecondaryFn) != 0;
    if (fn_now != fn_down) {
        fn_down = fn_now;
        openime_fn_edge(fn_now);
    }
    return event;
}

void openime_install_fn_monitor_objc(void) {
    // 1. CGEventTap：source 挂到主线程的 NSRunLoop common modes。
    CFMachPortRef tap = CGEventTapCreate(
        kCGSessionEventTap,
        kCGHeadInsertEventTap,
        kCGEventTapOptionListenOnly,
        CGEventMaskBit(kCGEventFlagsChanged),
        cg_callback,
        NULL);
    if (tap) {
        CGEventTapEnable(tap, true);
        CFRunLoopSourceRef src = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
        // 挂到主线程的 NSRunLoop（current loop 在主线程上 == NSRunLoop）。
        CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
        NSLog(@"[openIME] CGEventTap 已挂到主线程 run loop");
    } else {
        NSLog(@"[openIME] CGEventTap 创建失败（检查辅助功能权限）");
    }

    // 2. NSEvent global+local monitor（补充）。
    [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskFlagsChanged
        handler:^(NSEvent *event) { handle_event(event); }];
    [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskFlagsChanged
        handler:^(NSEvent *event) { handle_event(event); return event; }];

    NSLog(@"[openIME] Fn 监听已安装（CGEventTap@主线程 + NSEvent global+local）");
}
