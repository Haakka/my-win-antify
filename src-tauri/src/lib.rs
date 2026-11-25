#[path = "../../crates/core-service/src/atomic_file.rs"]
mod atomic_file;
mod autostart;
mod commands;
// Compile the same Win32 adapter enumeration into the desktop for offline use.
#[allow(dead_code, reason = "The desktop uses only the adapter list API")]
#[path = "../../crates/core-service/src/network_interfaces.rs"]
mod network_interfaces;
mod pipe_client;
mod service_launcher;

use std::{ffi::c_void, io, ptr};

use shared::RpcRequest;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Effect, EffectsBuilder},
    App, AppHandle, Manager, WindowEvent,
};

use pipe_client::PipeClient;

pub fn run() {
    let _single_instance = single_instance().expect("Unable to create MyWinAntify instance mutex");
    tauri::Builder::default()
        .setup(|app| {
            app.manage(PipeClient::start(app.handle().clone()));
            if let Some(window) = app.get_webview_window("main") {
                // Tauri applies this effect via window_vibrancy::apply_mica.
                let _ = window.set_effects(EffectsBuilder::new().effect(Effect::Mica).build());
            }
            install_tray(app)?;
            std::thread::spawn(|| {
                let _ = service_launcher::ensure_core_service_running();
            });
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
            commands::core_export_config_backup,
            commands::core_import_config_backup,
            commands::core_export_rules_json,
            commands::core_import_rules_json,
            commands::core_get_traffic_history,
            commands::core_clear_traffic_history,
            commands::core_get_current_wifi,
            commands::core_get_coexistence_conflicts,
            commands::core_dismiss_coexistence_conflict,
            commands::core_collect_diagnostic_bundle,
            commands::core_ping,
            commands::core_get_status,
            commands::core_start_engine,
            commands::core_stop_engine,
            commands::core_get_config,
            commands::core_save_config,
            commands::core_restore_config_backup,
            commands::core_set_active_profile,
            commands::core_get_connections,
            commands::core_clear_connections,
            commands::core_list_running_processes,
            commands::core_list_network_interfaces,
            commands::core_test_proxy,
            commands::core_get_activity_logs,
            commands::core_clear_activity_logs,
            commands::core_pipe_state,
            commands::core_launch_service,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MyWinAntify");
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attributes: *const c_void, initial_owner: i32, name: *const u16) -> isize;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn ShowWindow(window: isize, command: i32) -> i32;
    fn SetForegroundWindow(window: isize) -> i32;
}

struct InstanceMutex(isize);

impl Drop for InstanceMutex {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn single_instance() -> io::Result<InstanceMutex> {
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const SW_RESTORE: i32 = 9;
    let name: Vec<_> = "Local\\MyWinAntifySingleInstance\0"
        .encode_utf16()
        .collect();
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    let status = unsafe { GetLastError() };
    if handle == 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let instance = InstanceMutex(handle);
    if status == ERROR_ALREADY_EXISTS {
        let title: Vec<_> = "MyWinAntify\0".encode_utf16().collect();
        let window = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
        if window != 0 {
            unsafe {
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
            }
        }
        drop(instance);
        std::process::exit(0);
    }
    Ok(instance)
}

fn install_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动代理", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止代理", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 MyWinAntify", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &start, &stop, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("MyWinAntify")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "start" => engine_request(app, RpcRequest::StartEngine),
            "stop" => engine_request(app, RpcRequest::StopEngine),
            "quit" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(1200),
                        app.state::<PipeClient>().request(RpcRequest::StopEngine),
                    )
                    .await;
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn engine_request(app: &AppHandle, request: RpcRequest) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = app.state::<PipeClient>().request(request).await {
            tracing::warn!(%error, "Tray proxy command failed");
        }
    });
}
