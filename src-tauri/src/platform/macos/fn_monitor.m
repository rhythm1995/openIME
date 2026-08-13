// Fn 键监听（ObjC）：CGEventTap source 挂到主线程 run loop（common modes）。
// 之前在专用线程上注册 CGEventTap 收不到事件——CGEventTap 的 source 必须
// 挂在主线程的 NSRunLoop 上才能被正确 pump。
// 同时保留 NSEvent global+local monitor 做补充。
//
// R9：Hold+Fn 短按补发 🌐 + 吞键。
// - 补发事件是一对 kCGEventFlagsChanged（keycode 63），两条都写 kOpenimeRepostMagic。
// - 自捕获主过滤器：REPOST_IGNORE_MS 内忽略 keyCode 63 边沿；user-data 为辅。
// - Hold+Fn 时 g_fn_tap_consume=true → 吞 keyCode 63 的 flagsChanged；Toggle 不吞。

#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreFoundation/CoreFoundation.h>
#include <stdatomic.h>
#include <sys/time.h>

extern void openime_fn_edge(bool pressed);

static bool fn_down = false;
static CFMachPortRef g_tap = NULL;

// 补发事件 user-data magic（'OIME'）。
static const int64_t kOpenimeRepostMagic = 0x4F494D45;
static const uint64_t kRepostIgnoreMs = 60;
static _Atomic uint64_t g_ignore_until_ms = 0;
static _Atomic bool g_fn_tap_consume = false;

static uint64_t monotonic_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (uint64_t)tv.tv_sec * 1000 + (uint64_t)tv.tv_usec / 1000;
}

static bool is_repost(CGEventRef e) {
    return CGEventGetIntegerValueField(e, kCGEventSourceUserData) == kOpenimeRepostMagic;
}

static bool in_ignore_window(void) {
    return monotonic_ms() < atomic_load(&g_ignore_until_ms);
}

// NSEvent monitor 的 flagsChanged 处理（同一 ignore window）。
static void handle_event(NSEvent *event) {
    if (in_ignore_window()) return; // 补发窗口内忽略（自捕获）。
    NSUInteger keycode = event.keyCode;
    // 只处理 Fn 自身（keyCode=63）的 flagsChanged。
    if (keycode != 63) return;
    NSUInteger flags = event.modifierFlags;
    bool fn_now = (flags & NSEventModifierFlagFunction) != 0;
    if (fn_now != fn_down) {
        fn_down = fn_now;
        openime_fn_edge(fn_now);
    }
}

// CGEventTap 回调（C 函数）。
static CGEventRef cg_callback(CGEventTapProxy proxy, CGEventType type,
                               CGEventRef event, void *refcon) {
    (void)proxy; (void)refcon;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        if (g_tap) CGEventTapEnable(g_tap, true);
        return event;
    }
    if (type != kCGEventFlagsChanged) return event;
    int64_t keycode = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
    if (keycode != 63) return event;
    // 放行补发（magic userdata）与 ignore 窗口内的事件，不回调 Rust。
    if (is_repost(event) || in_ignore_window()) return event;
    CGEventFlags flags = CGEventGetFlags(event);
    bool fn_now = (flags & kCGEventFlagMaskSecondaryFn) != 0;
    if (fn_now != fn_down) {
        fn_down = fn_now;
        openime_fn_edge(fn_now);
    }
    // Hold+Fn：吞 keyCode 63 的 flagsChanged（系统 🌐 原功能走补发执行）。
    if (atomic_load(&g_fn_tap_consume)) return NULL;
    return event;
}

/// Rust 侧写 ignore deadline（post 前调用）。
void openime_arm_repost_ignore(void) {
    atomic_store(&g_ignore_until_ms, monotonic_ms() + kRepostIgnoreMs);
}

/// 补发一对 flagsChanged（keycode 63，down 带 SecondaryFn / up 清 flag）。
int openime_repost_fn(void) {
    openime_arm_repost_ignore();
    CGEventSourceRef src = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
    CGEventRef down = CGEventCreate(src);
    CGEventRef up = CGEventCreate(src);
    CGEventSetType(down, kCGEventFlagsChanged);
    CGEventSetType(up, kCGEventFlagsChanged);
    CGEventSetIntegerValueField(down, kCGKeyboardEventKeycode, 63);
    CGEventSetIntegerValueField(up, kCGKeyboardEventKeycode, 63);
    CGEventSetFlags(down, kCGEventFlagMaskSecondaryFn);
    CGEventSetFlags(up, 0);
    CGEventSetIntegerValueField(down, kCGEventSourceUserData, kOpenimeRepostMagic);
    CGEventSetIntegerValueField(up, kCGEventSourceUserData, kOpenimeRepostMagic);
    CGEventPost(kCGHIDEventTap, down);
    CGEventPost(kCGHIDEventTap, up);
    CFRelease(down); CFRelease(up); CFRelease(src);
    return 1;
}

/// 先写 ignore deadline，再下一圈 main runloop 再 post（禁止在 tap 回调栈上同步 post）。
void openime_schedule_repost_fn(void) {
    openime_arm_repost_ignore();
    CFRunLoopPerformBlock(CFRunLoopGetMain(), kCFRunLoopCommonModes, ^{
        openime_repost_fn();
    });
}

/// Rust 侧下发「是否吞 Fn 键」：hotkey==Fn && Hold 才吞。
void openime_set_fn_tap_consume(bool consume) {
    atomic_store(&g_fn_tap_consume, consume);
}

void openime_install_fn_monitor_objc(void) {
    // 1. CGEventTap：Default（可吞键）；失败退回 ListenOnly。source 挂主线程 NSRunLoop。
    CFMachPortRef tap = CGEventTapCreate(
        kCGSessionEventTap,
        kCGHeadInsertEventTap,
        kCGEventTapOptionDefault,
        CGEventMaskBit(kCGEventFlagsChanged),
        cg_callback,
        NULL);
    if (tap) {
        g_tap = tap;
        CGEventTapEnable(tap, true);
        CFRunLoopSourceRef src = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
        NSLog(@"[openIME] CGEventTap（Default，可吞键）已挂到主线程 run loop");
    } else {
        tap = CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionListenOnly,
            CGEventMaskBit(kCGEventFlagsChanged),
            cg_callback,
            NULL);
        if (tap) {
            g_tap = tap;
            CGEventTapEnable(tap, true);
            CFRunLoopSourceRef src = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
            CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
            NSLog(@"[openIME] CGEventTap（ListenOnly）已挂到主线程 run loop（吞键不可用）");
        } else {
            NSLog(@"[openIME] CGEventTap 创建失败（检查辅助功能/输入监控权限）");
            atomic_store(&g_fn_tap_consume, false);
        }
    }

    // 2. NSEvent global+local monitor（补充）。
    [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskFlagsChanged
        handler:^(NSEvent *event) { handle_event(event); }];
    [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskFlagsChanged
        handler:^(NSEvent *event) { handle_event(event); return event; }];

    NSLog(@"[openIME] Fn 监听已安装（CGEventTap@主线程 + NSEvent global+local）");
}
