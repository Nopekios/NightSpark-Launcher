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
mod watermark;

pub use error::AppError;

#[tauri::command]
fn watermark_info() -> watermark::Watermark {
    watermark::info()
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            commands::list_java_runtimes,
            watermark_info
        ])
        .setup(|_app| {
            // 旧配置目录一次性迁移（%APPDATA%\Love-Launcher -> %APPDATA%\NightSpark）。
            // 必须挂在 setup：它早于窗口创建和前端 invoke，能保证配置、账号、实例
            // 在第一次被读取之前就已经落到新目录；放到前端首次调用里就晚了。
            // 迁移函数内部吞掉所有错误（只写 stderr），旧目录异常不会让启动失败。
            config::migrate_legacy_home();
            watermark::write_boot_stamp();
            Ok(())
        });

    if let Err(err) = builder.run(tauri::generate_context!()) {
        eprintln!("NightSpark 启动失败: {err}");
    }
}
