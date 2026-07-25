mod commands;
mod credentials;
mod fsx;
mod targets;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            commands::bootstrap::get_bootstrap,
            commands::targets::list_targets,
            commands::targets::apply_target,
            commands::targets::apply_all_targets,
            commands::diagnostics::ping,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
