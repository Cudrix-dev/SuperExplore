fn main() {
    println!("cargo:rerun-if-changed=SuperExplore.manifest");
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("SuperExplore.manifest"));
    let commands = tauri_build::AppManifest::new().commands(&[
        "app_status",
        "show_splash",
        "finish_startup",
        "search_files",
        "hide_main",
        "minimize_main",
        "open_result",
        "copy_path",
    ]);
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(windows)
            .app_manifest(commands),
    )
    .expect("failed to configure the SuperExplore Windows build");
}
