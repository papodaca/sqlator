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
            commands::parse_connection_url,
            commands::test_connection_with_ssh,
            commands::check_keyring_available,
            commands::get_storage_mode,
            commands::set_storage_mode,
            commands::vault_exists,
            commands::is_vault_locked,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::get_vault_settings,
            commands::save_vault_settings,
            commands::get_groups,
            commands::save_group,
            commands::update_group,
            commands::delete_group,
            commands::move_connection_to_group,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
