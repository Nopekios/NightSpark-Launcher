mod auth;
mod commands;
mod config;
mod download;
mod error;
mod hash;
mod instance;
mod java;
mod launch;
mod market;
mod net;

pub use error::AppError;

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::get_java_list,
            commands::list_instances,
            commands::create_instance,
            commands::delete_instance, commands::import_local_versions,
        commands::sync_instance_files,
                commands::import_modpack,
            commands::download_pending_mods,
            commands::install_instance_loader, commands::verify_instance,
            commands::start_login,
            commands::exchange_code,
            commands::get_account,
            commands::refresh_account,
            commands::logout,
            commands::get_config,
            commands::save_config,
            commands::fetch_version_list,
            commands::install_version,
            commands::is_version_installed,
            commands::launch_instance,
            commands::kill_game_process, commands::read_latest_log,
            commands::search_market,
            commands::list_market_files,
            commands::install_market_file,
            commands::install_market_modpack,
            commands::install_java,
            commands::list_java_runtimes
        ]);

    if let Err(err) = builder.run(tauri::generate_context!()) {
        eprintln!("Love-Launcher 启动失败: {err}");
    }
}
