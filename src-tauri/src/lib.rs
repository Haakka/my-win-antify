mod commands;
// Compile the same Win32 adapter enumeration into the desktop for offline use.
#[path = "../../crates/core-service/src/network_interfaces.rs"]
mod network_interfaces;
mod pipe_client;
mod service_launcher;

use shared::RpcRequest;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Effect, EffectsBuilder},
    App, AppHandle, Manager, WindowEvent,
};

use pipe_client::PipeClient;

pub fn run() {
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
            commands::core_ping,
            commands::core_get_status,
            commands::core_start_engine,
            commands::core_stop_engine,
            commands::core_get_config,
            commands::core_save_config,
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
