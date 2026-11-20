#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod shell;

use serde::Serialize;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use superexplore::{monitor, Index, SearchResponse, Status};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use window_vibrancy::{apply_acrylic, apply_mica};
use windows::Win32::{
    Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    },
    UI::Controls::MARGINS,
};

#[derive(Default)]
struct Desktop {
    started: AtomicBool,
    shortcut: AtomicBool,
    material: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    #[serde(flatten)]
    index: Status,
    shortcut_available: bool,
    material_available: bool,
}

#[tauri::command]
fn app_status(index: State<'_, Index>, desktop: State<'_, Desktop>) -> AppStatus {
    AppStatus {
        index: index.status(),
        shortcut_available: desktop.shortcut.load(Ordering::Relaxed),
        material_available: desktop.material.load(Ordering::Relaxed),
    }
}

#[tauri::command]
fn show_splash(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "splash" {
        return Err("启动画面窗口不可用".into());
    }
    window.show().map_err(|error| error.to_string())
}

fn show_main(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

#[tauri::command]
fn finish_startup(
    window: WebviewWindow,
    app: AppHandle,
    index: State<'_, Index>,
    desktop: State<'_, Desktop>,
) -> Result<(), String> {
    if window.label() != "splash" || !index.is_ready() {
        return Err("正在准备初始索引".into());
    }
    show_main(&app).map_err(|error| error.to_string())?;
    desktop.started.store(true, Ordering::Release);
    window.close().map_err(|error| error.to_string())
}

#[tauri::command]
async fn search_files(query: String, index: State<'_, Index>) -> Result<SearchResponse, String> {
    let index = index.inner().clone();
    tokio::task::spawn_blocking(move || index.search(&query))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_main(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
fn minimize_main(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|error| error.to_string())
}

fn apply_material(window: &WebviewWindow) -> bool {
    if let Ok(hwnd) = window.hwnd() {
        let margins = MARGINS {
            cxLeftWidth: -1,
            ..Default::default()
        };
        unsafe {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                std::ptr::from_ref(&DWMWCP_ROUND).cast(),
                std::mem::size_of_val(&DWMWCP_ROUND) as u32,
            );
        }
    }
    apply_mica(window, Some(true))
        .or_else(|_| apply_acrylic(window, Some((20, 24, 33, 185))))
        .is_ok()
}

fn local_path(path: String) -> Result<PathBuf, String> {
    use std::path::{Component, Prefix};
    let path = PathBuf::from(path);
    let valid = path.is_absolute()
        && path.components().next().is_some_and(|component| {
            let Component::Prefix(prefix) = component else {
                return false;
            };
            match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => true,
                Prefix::Verbatim(name) => name.to_string_lossy().starts_with("Volume{"),
                _ => false,
            }
        });
    if valid {
        Ok(path)
    } else {
        Err("目标不是本地磁盘路径".into())
    }
}

#[tauri::command]
async fn open_result(path: String, containing_folder: bool) -> Result<(), String> {
    let path = local_path(path)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("explorer-shell".into())
        .spawn(move || {
            let target = if containing_folder {
                path.parent().unwrap_or(&path)
            } else {
                &path
            };
            let _ = sender.send(
                shell::open(target).map_err(|error| format!("资源管理器无法打开此项目 · {error}")),
            );
        })
        .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}

#[tauri::command]
fn copy_path(app: AppHandle, path: String) -> Result<(), String> {
    app.clipboard()
        .write_text(path)
        .map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Index::default())
        .manage(Desktop::default())
        .setup(|app| {
            let main = app
                .get_webview_window("main")
                .expect("configured main window");
            let desktop = app.state::<Desktop>();
            desktop
                .material
                .store(apply_material(&main), Ordering::Relaxed);

            let shortcut = app
                .global_shortcut()
                .on_shortcut("Alt+Space", |app, _, event| {
                    if event.state != ShortcutState::Pressed
                        || !app.state::<Desktop>().started.load(Ordering::Acquire)
                    {
                        return;
                    }
                    if let Some(main) = app.get_webview_window("main") {
                        if main.is_visible().unwrap_or(false)
                            && !main.is_minimized().unwrap_or(false)
                        {
                            let _ = main.hide();
                        } else {
                            let _ = show_main(app);
                        }
                    }
                });
            desktop.shortcut.store(shortcut.is_ok(), Ordering::Relaxed);

            let open = MenuItem::with_id(app, "show", "打开 SuperExplore", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 SuperExplore", true, None::<&str>)?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("SuperExplore")
                .menu(&Menu::with_items(app, &[&open, &quit])?)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" if app.state::<Desktop>().started.load(Ordering::Acquire) => {
                        let _ = show_main(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            let index = app.state::<Index>().inner().clone();
            std::thread::Builder::new()
                .name("ntfs-index".into())
                .spawn(move || monitor(index))?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            show_splash,
            finish_startup,
            search_files,
            hide_main,
            minimize_main,
            open_result,
            copy_path
        ])
        .run(tauri::generate_context!())
        .expect("could not start SuperExplore");
}
