fn main() {
    tauri_build::build();

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=dylib=objc");

        println!("cargo:rerun-if-changed=src/platform/macos/fn_monitor.m");
        println!("cargo:rerun-if-changed=src/platform/macos/app_focus.m");
        // 编译 Fn 键监听的 ObjC 文件（NSEvent monitor + flagsChanged）。
        cc::Build::new()
            .file("src/platform/macos/fn_monitor.m")
            .compile("fn_monitor");
        // 编译前台 app 激活 + overlay HUD 的 ObjC 文件。
        cc::Build::new()
            .file("src/platform/macos/app_focus.m")
            .compile("app_focus");
    }
}
