pub mod codex_sessions;
mod active_groups;
mod commands;
mod credentials;
mod fsx;
mod logx;
mod presence;
mod providers;
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
            fit_main_window_to_screen(app);
            crate::presence::spawn_heartbeat();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            providers::newapi::newapi_connect,
            providers::newapi::newapi_bootstrap,
            providers::newapi::newapi_provision,
            providers::newapi::newapi_pricing,
            providers::newapi::newapi_status,
            providers::newapi::newapi_usage,
            commands::auth::save_remembered_login,
            commands::auth::load_remembered_login,
            commands::auth::clear_remembered_login,
            commands::auth::save_remembered_station,
            commands::auth::load_remembered_station,
            commands::auth::clear_remembered_station,
            commands::registration::start_registration_challenge,
            commands::registration::registration_challenge_status,
            commands::registration::cancel_registration_challenge,
            commands::registration::register_niko_account,
            commands::bootstrap::get_bootstrap,
            commands::targets::list_targets,
            commands::targets::detect_active_groups,
            commands::targets::detect_effective_selections,
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
            commands::diagnostics::benchmark_group,
            commands::payment::open_cashier,
            commands::payment::close_cashier,
            commands::process::check_process,
            commands::process::check_all_processes,
            commands::process::restart_target,
            commands::process::close_target,
            commands::snapshots::list_snapshots,
            commands::snapshots::restore_snapshot,
            commands::codex_sessions::scan_codex_session_inventory,
            commands::codex_sessions::normalize_codex_session_storage,
            commands::codex_sessions::normalize_codex_session_storage_selected,
            commands::codex_sessions::open_codex_thread,
            crate::presence::fetch_usage_stats,
            autostart_enable,
            autostart_disable,
            autostart_is_enabled,
            relaunch_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ─── 窗口尺寸自适应 ─────────────────────────────────────────────────────────

/// 把主窗口拉高到屏幕允许的最大高度（留出菜单栏/程序坞余量），
/// 屏幕不够高时不强行超出，避免窗口跑到屏幕外。
fn fit_main_window_to_screen(app: &tauri::App) {
    use tauri::{LogicalSize, Manager};

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);

    // 目标高度：偏好 900，但绝不超过屏幕高度的 80%（用户要求），下限 560
    let preferred = 900.0_f64;
    let available = (screen.height * 0.8).max(560.0);
    let height = preferred.min(available);
    // 宽度同理：偏好 1180，不超出屏幕宽度的 90%
    let width = 1180.0_f64.min((screen.width * 0.9).max(800.0));

    let _ = window.set_size(LogicalSize::new(width, height));
    // 居中并置顶聚焦：避免窗口出现在屏幕外或藏在其他窗口后面
    let _ = window.center();
    let _ = window.show();
    let _ = window.set_focus();
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

/// 安装完更新包后重启当前应用。macOS 替换完成后必须重启才会切到新版本。
#[tauri::command]
fn relaunch_app(app: tauri::AppHandle) {
    tauri::process::restart(&app.env());
}
