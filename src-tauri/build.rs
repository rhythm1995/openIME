fn main() {
    tauri_build::build();

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=dylib=objc");

        // 编译 Fn 键监听的 ObjC 文件（NSEvent monitor + flagsChanged）。
        cc::Build::new()
            .file("src/platform/macos/fn_monitor.m")
            .compile("fn_monitor");
        // 编译前台 app 激活的 ObjC 文件。
        cc::Build::new()
            .file("src/platform/macos/app_focus.m")
            .compile("app_focus");
    }
}
