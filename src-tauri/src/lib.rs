pub mod codex_sessions;
mod active_groups;
mod commands;
mod credentials;
mod fsx;
mod logx;
mod targets;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .manage(commands::registration::RegistrationState::default())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            commands::auth::save_remembered_login,
            commands::auth::load_remembered_login,
            commands::auth::clear_remembered_login,
            commands::registration::start_registration_challenge,
            commands::registration::registration_challenge_status,
            commands::registration::cancel_registration_challenge,
            commands::registration::register_niko_account,
            commands::bootstrap::get_bootstrap,
            commands::targets::list_targets,
            commands::targets::detect_active_groups,
            commands::targets::apply_target,
            commands::targets::apply_all_targets,
            commands::targets::check_drift_cmd,
            commands::targets::check_all_drift,
            commands::targets::test_connectivity,
            commands::targets::restore_target_defaults,
            commands::diagnostics::ping,
            commands::diagnostics::verify_targets,
            commands::diagnostics::ping_diag,
            commands::diagnostics::export_log,
            commands::diagnostics::probe_compat,
            commands::payment::open_cashier,
            commands::payment::close_cashier,
            commands::process::check_process,
            commands::process::check_all_processes,
            commands::process::restart_target,
            commands::snapshots::list_snapshots,
            commands::snapshots::restore_snapshot,
            commands::codex_sessions::scan_codex_session_inventory,
            commands::codex_sessions::normalize_codex_session_storage,
            commands::codex_sessions::normalize_codex_session_storage_selected,
            commands::codex_sessions::open_codex_thread,
            autostart_enable,
            autostart_disable,
            autostart_is_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ─── 托盘图标 (E8-1) ────────────────────────────────────────────────────────

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::menu::{Menu, MenuItem};

    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::new()
        // 单色版 mark：macOS 菜单栏按模板图处理，自动适配深浅色
        .icon(tauri::include_image!("icons/tray-mono.png"))
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("Niko")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

// ─── 开机自启命令 (E8-1) ────────────────────────────────────────────────────

#[tauri::command]
async fn autostart_enable(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().enable().map_err(|e| e.to_string())
}

#[tauri::command]
async fn autostart_disable(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().disable().map_err(|e| e.to_string())
}

#[tauri::command]
async fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}
