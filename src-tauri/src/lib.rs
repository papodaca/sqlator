mod commands;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::new().expect("Failed to initialize app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_connections,
            commands::save_connection,
            commands::update_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::connect_database,
            commands::disconnect_database,
            commands::execute_query,
            commands::get_query,
            commands::save_query,
            commands::get_theme,
            commands::save_theme,
            commands::list_ssh_hosts,
            commands::create_ssh_tunnel,
            commands::close_ssh_tunnel,
            commands::get_active_tunnels,
            commands::get_ssh_profiles,
            commands::save_ssh_profile,
            commands::update_ssh_profile,
            commands::delete_ssh_profile,
            commands::connections_using_ssh_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
