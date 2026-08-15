fn main() {
    // macOS 无 TSF DLL（仅 Windows 的 build_tsf_dll 产出），但 tauri_build::build() 会
    // 校验 tauri.conf.json 声明的 resources 文件存在；建空占位通过校验，Windows 侧
    // build_tsf_dll 产出的真 DLL 会覆盖它。占位目录已 .gitignore（src-tauri/ime/）。
    #[cfg(target_os = "macos")]
    {
        let dll = std::path::Path::new("ime/OpenImeTsf.dll");
        if !dll.exists() {
            std::fs::create_dir_all("ime").ok();
            std::fs::write("ime/OpenImeTsf.dll", []).ok();
        }
    }

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

    // R11 阶段 A：编译 TSF TIP DLL（OpenImeTsf.dll → src-tauri/ime/）。
    // 用 cc::windows_registry 定位 cl.exe（本机/CI 均只有 VS BuildTools，无独立 CMake，
    // 故不走设计文档的 CMakeLists，用 cl 一次编译+链接，等价 /MT /W4 /EHsc /std:c++17）。
    // 工具缺失时 warn + 跳过：宿主侧 status=NotInstalled → 走 R7 插入，不阻塞构建。
    #[cfg(target_os = "windows")]
    {
        for f in [
            "dllmain.cpp",
            "class_factory.cpp",
            "text_service.cpp",
            "edit_session.cpp",
            "ipc_server.cpp",
            "registry.cpp",
            "guids.h",
            "common.h",
            "ipc_server.h",
            "edit_session.h",
            "text_service.h",
            "OpenImeTsf.def",
        ] {
            println!("cargo:rerun-if-changed=windows-ime/{f}");
        }
        if let Err(e) = build_tsf_dll() {
            println!("cargo:warning=OpenImeTsf.dll 构建跳过（{e}）：TSF 上屏不可用，回退 enigo/粘贴（R7）");
        }
    }
}

#[cfg(target_os = "windows")]
fn build_tsf_dll() -> Result<(), String> {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("windows-ime");
    let out_dir = manifest.join("ime");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let dll = out_dir.join("OpenImeTsf.dll");

    let sources = [
        "dllmain.cpp",
        "class_factory.cpp",
        "text_service.cpp",
        "edit_session.cpp",
        "ipc_server.cpp",
        "registry.cpp",
    ];

    // cl.exe 一把梭：/LD = 编译并链接 DLL；/DEF 导出四个 COM 入口。
    let tool = cc::windows_registry::find_tool("x86_64-pc-windows-msvc", "cl.exe")
        .ok_or("找不到 MSVC cl.exe（VS C++ BuildTools 未安装）")?;
    let mut objs_dir = out_dir.join("obj");
    std::fs::create_dir_all(&objs_dir).map_err(|e| e.to_string())?;
    objs_dir.push(""); // /Fo 需以分隔符结尾（对象输出目录）

    let mut cmd = Command::new(tool.path());
    cmd.current_dir(&src)
        .arg("/nologo")
        .arg("/LD")
        .arg("/MT")
        .arg("/W4")
        .arg("/EHsc")
        .arg("/std:c++17")
        .arg("/DUNICODE")
        .arg("/D_UNICODE")
        .arg("/DNDEBUG")
        .args(sources.iter().map(|s| src.join(s)))
        .arg(format!("/Fo:{}", objs_dir.display()))
        .arg(format!("/Fe:{}", dll.display()))
        .arg(format!("/Fd:{}", out_dir.join("OpenImeTsf.pdb").display()))
        // /DEF 是链接器选项，必须在 /link 之后（cl 不透传给链接器）。
        .arg("/link")
        .arg("/DEF:OpenImeTsf.def")
        .arg("ole32.lib")
        .arg("advapi32.lib")
        .arg("user32.lib");
    for (k, v) in tool.env() {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let merged = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .replace('\r', "")
        .replace('\n', " | ");
        return Err(format!("cl.exe 失败：{merged}"));
    }
    if !dll.exists() {
        return Err("未产出 OpenImeTsf.dll".into());
    }
    Ok(())
}
