//! Tauri 命令层，只做裸注册，不掺业务逻辑。

use crate::auth;
use crate::config;
use crate::instance;
use crate::market;
use crate::AppError;

use tauri::{Emitter, Manager};

// 登录窗口的 label；重复点登录时先关掉旧窗口。
const MSA_LOGIN_WINDOW: &str = "msa-login";
const MSA_CODE_EVENT: &str = "msa-code";
const MSA_ERROR_EVENT: &str = "msa-error";
// 登录链路进度事件，payload 是一句给用户看的阶段文案。
const LOGIN_PROGRESS_EVENT: &str = "login-progress";
// 加载器安装进度事件，payload 是 { stage, done, total }。
const LOADER_PROGRESS_EVENT: &str = "loader-progress";

#[tauri::command]
pub async fn ping() -> Result<String, AppError> {
    Ok("pong".into())
}

#[tauri::command]
pub async fn get_java_list() -> Result<Vec<config::JavaInfo>, AppError> {
    config::detect_java()
}

#[tauri::command]
pub async fn list_instances() -> Result<Vec<instance::Instance>, AppError> {
    instance::list_instances()
}

#[tauri::command]
pub async fn create_instance(name: String, version: String) -> Result<instance::Instance, AppError> {
    instance::create_instance(&name, &version)
}

#[tauri::command]
pub async fn delete_instance(id: String) -> Result<(), AppError> {
    instance::delete_instance(&id)
}

#[tauri::command]
pub async fn import_local_versions() -> Result<Vec<instance::ImportedInstance>, AppError> {
    instance::import_installed_versions()
}

#[tauri::command]
pub async fn sync_instance_files(instance_id: String) -> Result<instance::CarryReport, AppError> {
    instance::sync_instance_from_shared(&instance_id)
}

/// 导入整合包文件（zip），识别 PCL / HMCL 结构后解压成实例。
#[tauri::command]
pub async fn import_modpack(
    app: tauri::AppHandle,
    path: String,
) -> Result<instance::modpack::ImportReport, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress {
                stage: stage.to_string(),
                done,
                total,
            },
        );
    };
    tauri::async_runtime::spawn_blocking(move || {
        instance::modpack::import_modpack_reported(&path, &sink)
    })
    .await
    .map_err(|err| AppError::Internal(format!("导入整合包时后台任务异常：{err}")))?
}

/// 补齐 CurseForge / Modrinth 整合包留下的待下载清单（有直链的才下）。
#[tauri::command]
pub async fn download_pending_mods(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<instance::modpack::PendingReport, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress {
                stage: stage.to_string(),
                done,
                total,
            },
        );
    };
    instance::modpack::download_pending_reported(&instance_id, &sink).await
}

#[tauri::command]
pub async fn install_instance_loader(
    app: tauri::AppHandle,
    instance_id: String,
    loader: String,
    loader_version: Option<String>,
    mc_version: Option<String>,
    java_path: Option<String>,
) -> Result<instance::loader::LoaderReport, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            LOADER_PROGRESS_EVENT,
            serde_json::json!({ "stage": stage, "done": done, "total": total }),
        );
    };
    let mut report =
        instance::loader::install(&instance_id, &loader, loader_version, mc_version, java_path, &sink)
            .await?;
    // 装完把实例指到带加载器的版本 id，否则启动还是原版。
    if let Err(err) = instance::set_version(&instance_id, &report.target_version) {
        report
            .messages
            .push(format!("实例版本没能切到 {}：{err}", report.target_version));
    }
    Ok(report)
}

/// 校验实例文件完整性：按版本 json 的 SHA-1 核对 client jar 与 libraries。
#[tauri::command]
pub async fn verify_instance(instance_id: String) -> Result<instance::verify::VerifyReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || instance::verify::verify_instance(&instance_id))
        .await
        .map_err(|err| AppError::Internal(format!("校验任务没能跑完：{err}")))?
}

fn emit_progress(app: &tauri::AppHandle, message: &str) {
    let _ = app.emit(LOGIN_PROGRESS_EVENT, message.to_string());
}

/// 账号库变化（登录、切换、移除、后台刷新）统一广播，前端据此刷新账号列表。
const ACCOUNTS_CHANGED_EVENT: &str = "accounts-changed";

fn emit_accounts_changed(app: &tauri::AppHandle) {
    if let Ok(list) = auth::list_accounts() {
        let _ = app.emit(ACCOUNTS_CHANGED_EVENT, list);
    }
}

/// 后台静默续期：每 5 分钟扫一遍账号库，把剩余有效期不足 5 分钟的账号刷新掉。
/// 这样用户从不需要手点"刷新令牌"，只有在 refresh_token 本身失效时才需要重新登录。
pub async fn auto_refresh_accounts_loop(app: tauri::AppHandle) {
    loop {
        let store = auth::load_store();
        if let Ok(store) = store {
            let mut changed = false;
            for account in store.accounts.clone() {
                if account.refresh_token.is_none() || !auth::needs_refresh(&account) {
                    continue;
                }
                match auth::refresh_account(&account).await {
                    Ok(fresh) => {
                        if auth::update_account_silently(&fresh).is_ok() {
                            changed = true;
                        }
                    }
                    // 刷新失败（网络抖动或 refresh_token 吊销）不打断循环，下次再试。
                    Err(err) => eprintln!("自动刷新令牌失败（{}）: {err}", account.name),
                }
            }
            if changed {
                emit_accounts_changed(&app);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
    }
}

// 回调地址固定是 https://login.live.com/oauth20_desktop.srf，微软把 code 拼在 query 上。
fn parse_callback(url: &tauri::Url) -> Option<Result<String, String>> {
    let is_callback =
        url.host_str() == Some("login.live.com") && url.path() == "/oauth20_desktop.srf";
    if !is_callback {
        return None;
    }
    let mut code = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.to_string()),
            "error" => error = Some(value.to_string()),
            _ => {}
        }
    }
    match (code, error) {
        (Some(code), _) => Some(Ok(code)),
        (None, Some(error)) => Some(Err(format!("微软授权失败: {error}"))),
        (None, None) => Some(Err("回调 URL 里既没有 code 也没有 error".to_string())),
    }
}

fn finish_login(app: &tauri::AppHandle, result: Result<String, String>) {
    match result {
        Ok(code) => {
            let _ = app.emit(MSA_CODE_EVENT, code);
        }
        Err(error) => {
            let _ = app.emit(MSA_ERROR_EVENT, error);
        }
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(MSA_LOGIN_WINDOW) {
            let _ = window.close();
        }
    });
}

// 走 live.com 公开客户端授权码流：webview 加载授权页，登录后微软 302 到 oauth20_desktop.srf，
// 在导航阶段拦下 URL 取 code，不真正加载回调页。
#[tauri::command]
pub async fn start_login(app: tauri::AppHandle) -> Result<(), AppError> {
    if let Some(old) = app.get_webview_window(MSA_LOGIN_WINDOW) {
        let _ = old.close();
    }
    let url = auth::authorize_url()
        .parse::<tauri::Url>()
        .map_err(|e| AppError::Config(format!("授权 URL 非法: {e}")))?;

    let nav_app = app.clone();
    tauri::WebviewWindowBuilder::new(&app, MSA_LOGIN_WINDOW, tauri::WebviewUrl::External(url))
        .title("Microsoft 登录")
        .inner_size(520.0, 760.0)
        .center()
        .on_navigation(move |url| {
            if let Some(result) = parse_callback(url) {
                finish_login(&nav_app, result);
                return false;
            }
            true
        })
        .on_page_load(move |webview, payload| {
            if let Some(result) = parse_callback(payload.url()) {
                finish_login(webview.app_handle(), result);
            }
        })
        .build()
        .map_err(|e| AppError::Config(format!("创建登录窗口失败: {e}")))?;

    Ok(())
}

#[tauri::command]
pub async fn exchange_code(app: tauri::AppHandle, code: String) -> Result<auth::AccountView, AppError> {
    emit_progress(&app, "正在用授权码换取微软令牌…");
    let msa = auth::exchange_code_for_token(&code).await?;

    emit_progress(&app, "正在通过 Xbox Live 认证…");
    let (xbl_token, _) = auth::xbox_live_authenticate(&msa.access_token).await?;

    emit_progress(&app, "正在申请 XSTS 授权…");
    let (xsts_token, uhs) = auth::xsts_authorize(&xbl_token).await?;

    emit_progress(&app, "正在登录 Minecraft 服务…");
    let (mc_token, expires_in) = auth::login_with_xbox(&uhs, &xsts_token).await?;

    emit_progress(&app, "正在读取 Minecraft 档案…");
    let (uuid, name) = auth::fetch_profile(&mc_token).await?;

    let account = auth::Account {
        uuid,
        name,
        access_token: Some(mc_token),
        refresh_token: msa.refresh_token,
        expires_at: Some(auth::now_secs() + expires_in as i64),
    };
    auth::save_account(&account)?;
    emit_progress(&app, "登录完成");
    // 其它窗口（侧栏）靠这个事件同步账号状态，多账号登录完也能立刻看到。
    emit_accounts_changed(&app);
    Ok(auth::AccountView::of(&account, true))
}

#[tauri::command]
pub async fn get_account(app: tauri::AppHandle) -> Result<Option<auth::AccountView>, AppError> {
    let _ = &app;
    Ok(auth::list_accounts()?.into_iter().find(|a| a.active))
}

#[tauri::command]
pub async fn list_accounts(app: tauri::AppHandle) -> Result<Vec<auth::AccountView>, AppError> {
    let _ = &app;
    auth::list_accounts()
}

#[tauri::command]
pub async fn switch_account(app: tauri::AppHandle, uuid: String) -> Result<auth::AccountView, AppError> {
    let view = auth::switch_account(&uuid)?;
    emit_accounts_changed(&app);
    Ok(view)
}

#[tauri::command]
pub async fn remove_account(app: tauri::AppHandle, uuid: String) -> Result<(), AppError> {
    auth::remove_account(&uuid)?;
    emit_accounts_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn refresh_account(app: tauri::AppHandle) -> Result<auth::AccountView, AppError> {
    let current = auth::load_account()?
        .ok_or_else(|| AppError::NotFound("本地没有已登录账号".to_string()))?;
    let refreshed = auth::refresh_account(&current).await?;
    auth::update_account_silently(&refreshed)?;
    emit_accounts_changed(&app);
    Ok(auth::AccountView::of(&refreshed, true))
}

#[tauri::command]
pub async fn logout(app: tauri::AppHandle) -> Result<(), AppError> {
    auth::clear_account()?;
    emit_accounts_changed(&app);
    Ok(())
}

use crate::download;
use crate::java;
use crate::launch;

const INSTALL_PROGRESS_EVENT: &str = "install-progress";

#[derive(serde::Serialize, Clone)]
pub struct InstallProgress {
    pub stage: String,
    pub done: u64,
    pub total: u64,
}

#[tauri::command]
pub async fn get_config() -> Result<config::LauncherConfig, AppError> {
    config::load_config()
}

#[tauri::command]
pub async fn save_config(cfg: config::LauncherConfig) -> Result<(), AppError> {
    config::save_config(&cfg)
}

#[tauri::command]
pub async fn fetch_version_list() -> Result<Vec<download::VersionEntry>, AppError> {
    let manifest = download::fetch_manifest().await?;
    Ok(manifest.versions)
}

#[tauri::command]
pub async fn install_version(
    app: tauri::AppHandle,
    version: String,
    with_assets: bool,
) -> Result<download::InstallReport, AppError> {
    let handle = app.clone();
    let progress = move |stage: &str, done: u64, total: u64| {
        let _ = handle.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress {
                stage: stage.to_string(),
                done,
                total,
            },
        );
    };
    download::install_version(&version, with_assets, &progress).await
}

#[tauri::command]
pub async fn is_version_installed(version: String) -> Result<bool, AppError> {
    Ok(download::is_installed(&version))
}

fn find_instance(id: &str) -> Result<instance::Instance, AppError> {
    instance::list_instances()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| AppError::NotFound(format!("找不到实例 {id}")))
}

#[tauri::command]
pub async fn launch_instance(
    id: String,
    java_path: Option<String>,
    memory_mb: Option<u32>,
    app: tauri::AppHandle,
) -> Result<launch::LaunchOutcome, AppError> {
    let target = find_instance(&id)?;
    let mut account = auth::load_account()?
        .ok_or_else(|| AppError::NotFound("还没有登录微软账号".to_string()))?;
    // 令牌临近过期时先续期，避免带着过期 token 启动被服务端踢回。
    if account.refresh_token.is_some() {
        if let Ok(fresh) = auth::get_valid_token(&account).await {
            account.access_token = Some(fresh);
        }
    }
    let saved = config::load_config()?;
    let preferred = java_path
        .filter(|p| !p.trim().is_empty())
        .or(saved.java_path.clone())
        .unwrap_or_default();
    // 启动前先把实例该有的东西补齐，界面上只留一个启动按钮
    let prepare_notes = auto_prepare(&app, &target, &preferred).await;
    // 准备阶段可能把实例版本切成带加载器的，重新取一次
    let target = find_instance(&id)?;
    // 导入的实例 / 我们解包装的 Forge 都没有 natives 目录，启动前补一遍
    download::ensure_natives(&target.version)?;
    let meta = download::load_merged_meta(&target.version)?;
    // 用户选的 Java 跟这个版本对不上时自动换一个（低版本必须 Java 8）。
    let (java_path, java_note) = resolve_java(&preferred, &meta);
    let cfg = launch::LaunchConfig {
        java_path,
        memory_mb: memory_mb.unwrap_or(saved.memory_mb),
        jvm_args: Vec::new(),
        game_args: Vec::new(),
    };
    let exit_app = app.clone();
    let mut outcome = launch::launch(
        &target,
        &meta,
        &account,
        &cfg,
        Some(Box::new(move |pid| {
            use tauri::Emitter;
            let _ = exit_app.emit("game-exit", pid);
        })),
    )?;
    outcome.java_note = java_note;
    outcome.prepare_notes = prepare_notes;
    Ok(outcome)
}

/// 挑一个能跑这个版本的 Java。
///
/// 优先用户选的那个；不合适（1.16.5 及以下必须 Java 8，新版本要不低于元数据
/// 要求）就从本机已装的里面挑最贴合的一个，挑不到就原样返回，让 launch 报错。
fn resolve_java(preferred: &str, meta: &download::VersionMeta) -> (String, Option<String>) {
    let Some(required) = meta.java_version.as_ref().map(|j| j.major_version) else {
        return (preferred.to_string(), None);
    };
    let fits = |major: u32| {
        if required <= 8 {
            major == required
        } else {
            major >= required
        }
    };

    if !preferred.is_empty() {
        if let Some(major) = config::probe_major(preferred) {
            if fits(major) {
                return (preferred.to_string(), None);
            }
        }
    }

    let mut candidates: Vec<(u32, String)> = config::detect_java()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|info| config::major_from_version(&info.version).map(|m| (m, info.path)))
        .filter(|(major, _)| fits(*major))
        .collect();
    candidates.sort_by_key(|(major, _)| *major);

    if let Some((major, path)) = candidates.into_iter().next() {
        let note = if preferred.is_empty() {
            format!("自动选用本机的 Java {major}")
        } else {
            format!(
                "{} 需要 Java {required}，原来选的 {} 是 Java {}，已自动换成 Java {major}（{path}）",
                meta.id,
                preferred,
                config::probe_major(preferred).unwrap_or(0)
            )
        };
        return (path, Some(note));
    }
    (preferred.to_string(), None)
}

#[tauri::command]
pub async fn install_java(
    app: tauri::AppHandle,
    major: u32,
) -> Result<java::JavaRuntime, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress {
                stage: stage.to_string(),
                done,
                total,
            },
        );
    };
    let runtime = java::install(major, &sink).await?;
    // 之前一个 Java 都没选过，就把它设成默认；选过的别乱改。
    let mut saved = config::load_config()?;
    if saved.java_path.iter().all(|p| p.trim().is_empty()) {
        saved.java_path = Some(runtime.path.clone());
        config::save_config(&saved)?;
    }
    Ok(runtime)
}

#[tauri::command]
pub async fn list_java_runtimes() -> Result<Vec<java::JavaRuntime>, AppError> {
    Ok(java::installed())
}

/// 结束指定 PID 的游戏进程（Windows 下连同子进程树）。
#[tauri::command]
pub async fn kill_game_process(pid: u32) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| AppError::Internal(format!("调用 taskkill 失败: {e}")))?;
        if !status.success() {
            return Err(AppError::Internal(format!("结束进程 {pid} 失败，可能已经退出")));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(AppError::Internal(
            "当前平台暂不支持结束游戏进程".to_string(),
        ))
    }
}

/// 启动前的自动准备：整合包文件、待补 mod、版本文件，缺什么自己补。
///
/// 界面上只留「启动游戏」一个按钮，这些步骤全在这里跑，进度走安装进度事件。
async fn auto_prepare(
    app: &tauri::AppHandle,
    target: &instance::Instance,
    preferred_java: &str,
) -> Vec<String> {
    let mut notes = Vec::new();
    let sink_app = app.clone();
    let sink = move |stage: &str, done: u64, total: u64| {
        use tauri::Emitter;
        let _ = sink_app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress {
                stage: stage.to_string(),
                total,
                done,
            },
        );
    };

    let game_dir = std::path::PathBuf::from(&target.game_dir);
    let mods_now = instance::verify::count_jars(&game_dir.join("mods"));

    if mods_now == 0 {
        if let Ok(version_dir) = download::version_dir(&target.version) {
            if version_dir.is_dir() {
                sink("整理整合包内容", 0, 1);
                match instance::carry_version_files(&game_dir, &version_dir, &target.version, false) {
                    Ok(report) if report.files > 0 => notes.push(format!(
                        "已把本机版本里的 {} 个文件搬进实例（mods {} 个）",
                        report.files, report.mods
                    )),
                    Ok(_) => {}
                    Err(err) => notes.push(format!("搬整合包文件没成功：{err}")),
                }
                sink("整理整合包内容", 1, 1);
            }
        }
    }

    let pending = game_dir.join(".nightspark").join("pending-mods.json");
    if pending.is_file() {
        match instance::modpack::download_pending_reported(&target.id, &sink).await {
            Ok(report) => {
                if report.downloaded > 0 {
                    notes.push(format!("补齐 {} 个 mod", report.downloaded));
                }
                if report.failed > 0 {
                    notes.push(format!("还有 {} 个 mod 没补上，日志里能看到是谁", report.failed));
                }
            }
            Err(err) => notes.push(format!("补 mods 没跑完：{err}")),
        }
    }

    if !download::is_installed(&target.version) {
        sink("准备版本文件", 0, 1);
        match download::install_version(&target.version, true, &sink).await {
            Ok(_) => notes.push(format!("{} 的版本文件已自动准备好", target.version)),
            Err(err) => notes.push(format!(
                "{} 的官方版本文件没下下来（{err}），启动大概率会失败",
                target.version
            )),
        }
        sink("准备版本文件", 1, 1);
    }

    if let Some(loader) = pack_loader(&target.game_dir) {
        if !has_loader_in_name(&target.version) {
            let java = download::load_local_meta(&target.version)
                .map(|meta| resolve_java(preferred_java, &meta).0)
                .unwrap_or_else(|_| preferred_java.to_string());
            sink("安装加载器", 0, 1);
            match instance::loader::install(&target.id, &loader, None, None, Some(java), &sink).await {
                Ok(report) => match instance::set_version(&target.id, &report.target_version) {
                    Ok(_) => notes.push(format!(
                        "整合包要的 {} 装好了，实例版本切到 {}",
                        loader, report.target_version
                    )),
                    Err(err) => notes.push(format!("{loader} 装好了但实例版本没切过去：{err}")),
                },
                Err(err) => notes.push(format!(
                    "{loader} 没装上（{err}），这次启动只能是原版"
                )),
            }
            sink("安装加载器", 1, 1);
        }
    }

    notes
}

/// 读整合包导入时记下的加载器要求（.nightspark/pack.json）。
fn pack_loader(game_dir: &str) -> Option<String> {
    let path = std::path::Path::new(game_dir)
        .join(".nightspark")
        .join("pack.json");
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("loader")
        .and_then(|inner| inner.as_str())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 实例版本名里带加载器标记就认为已经装过，不必重复跑 installer。
fn has_loader_in_name(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    ["forge", "fabric", "quilt"].iter().any(|key| lower.contains(key))
}

#[tauri::command]
pub async fn read_latest_log(id: String, max_lines: Option<usize>) -> Result<String, AppError> {
    let target = find_instance(&id)?;
    let path = std::path::Path::new(&target.game_dir)
        .join("logs")
        .join("latest.log");
    if !path.is_file() {
        return Ok(String::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let limit = max_lines.unwrap_or(200);
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(limit);
    Ok(lines[start..].join("\n"))
}

/// 搜资源市场：`kind` 取 mod / modpack / resourcepack / shader，`source` 取 curseforge / modrinth。
#[tauri::command]
pub async fn search_market(
    source: String,
    kind: String,
    query: String,
    page: u32,
) -> Result<Vec<market::MarketProject>, AppError> {
    market::search(&source, &kind, &query, page).await
}

#[tauri::command]
pub async fn list_market_files(
    source: String,
    project_id: String,
    kind: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> Result<Vec<market::MarketFile>, AppError> {
    market::files(
        &source,
        &project_id,
        &kind,
        game_version.as_deref().unwrap_or_default(),
        loader.as_deref().unwrap_or_default(),
    )
    .await
}

#[tauri::command]
pub async fn install_market_file(
    app: tauri::AppHandle,
    instance_id: String,
    file: market::MarketFile,
) -> Result<market::MarketInstallReport, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            serde_json::json!({ "stage": stage, "done": done, "total": total }),
        );
    };
    market::install_file(&instance_id, file, &sink).await
}

#[tauri::command]
pub async fn install_market_modpack(
    app: tauri::AppHandle,
    file: market::MarketFile,
) -> Result<instance::modpack::ImportReport, AppError> {
    let sink = move |stage: &str, done: u64, total: u64| {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            serde_json::json!({ "stage": stage, "done": done, "total": total }),
        );
    };
    market::install_modpack(file, &sink).await
}
