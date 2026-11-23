//! 资源市场：把 CurseForge 与 Modrinth 的「搜索 → 选文件 → 落盘」收敛成一套接口。
//!
//! 两条来源的脾气不一样：
//! - Modrinth 完全开放，`api.modrinth.com/v2` 直接能搜能下，文件自带 `sha1` 与直链；
//! - CurseForge 官方 API 要 `x-api-key`，没 key 走 `mod.mcimirror.top` 镜像即可，
//!   文件直链是公开 CDN，按 `fileId` 拼得出来（规则见 [`net::curseforge_cdn_url`]）。
//!
//! 所以这里的顺序统一是「镜像优先、官方兜底」，任一来源挂掉都不至于整页空着。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::download::{self, FileJob, ProgressFn};
use crate::error::AppError;
use crate::instance;
use crate::net;

/// CurseForge 里 Minecraft 的游戏 id。
const CF_GAME_ID: u32 = 432;
/// 一页多少条。
const PAGE_SIZE: u32 = 20;
/// Modrinth 官方接口。
const MODRINTH_API: &str = "https://api.modrinth.com/v2";
/// Modrinth 镜像（同 path 直接替换 host）。
const MODRINTH_MIRROR: &str = "https://mod.mcimirror.top/modrinth";
/// 单次请求超时（秒）。
const HTTP_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// 对外数据结构
// ---------------------------------------------------------------------------

/// 搜索列表里的一条项目。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketProject {
    /// `curseforge` 或 `modrinth`。
    pub source: String,
    /// 项目 id，取文件列表时要用。
    pub id: String,
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub icon: Option<String>,
    pub downloads: u64,
    pub author: String,
    /// `mod` / `modpack` / `resourcepack` / `shader`。
    pub kind: String,
}

/// 项目下的一个可下载文件。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketFile {
    /// 落盘用的文件名。
    pub name: String,
    /// 版本展示名。
    pub version: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub url: String,
    pub size: Option<u64>,
    pub sha1: Option<String>,
    /// 该落到实例里的哪个目录（`mods` / `resourcepacks` / `shaderpacks`）。
    pub folder: String,
}

/// 装完一个文件的结果。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketInstallReport {
    pub file_name: String,
    pub folder: String,
    pub path: String,
    /// 本地已有且校验通过，没真的下载。
    pub skipped: bool,
}

// ---------------------------------------------------------------------------
// 类型映射
// ---------------------------------------------------------------------------

/// 资源类型 → CurseForge `classId`。
fn cf_class_id(kind: &str) -> u32 {
    match kind {
        "modpack" => 4471,
        "resourcepack" => 12,
        "shader" => 6552,
        "world" => 17,
        "datapack" => 6945,
        _ => 6,
    }
}

/// 资源类型 → Modrinth `project_type`。
fn mr_project_type(kind: &str) -> &'static str {
    match kind {
        "modpack" => "modpack",
        "resourcepack" => "resourcepack",
        "shader" => "shader",
        "datapack" => "datapack",
        _ => "mod",
    }
}

/// 资源类型 → 实例内默认目录。
fn default_folder(kind: &str) -> &'static str {
    match kind {
        "modpack" => "",
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "world" => "saves",
        "datapack" => "datapacks",
        _ => "mods",
    }
}

/// 加载器名 → CurseForge `modLoaderType`；不认识返回 `None`（不加筛选）。
fn cf_loader_type(loader: &str) -> Option<u32> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "liteloader" => Some(3),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// CurseForge 文件里已知的加载器名，用来把 `gameVersions` 拆成「游戏版本 + 加载器」。
const CF_LOADER_NAMES: [&str; 6] = ["Forge", "NeoForge", "Fabric", "Quilt", "LiteLoader", "Rift"];

// ---------------------------------------------------------------------------
// HTTP 小工具
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .user_agent("NightSpark/0.1")
        .build()
        .map_err(|err| AppError::Http(format!("创建 HTTP 客户端失败：{err}")))
}

/// 发一个 GET 拿 JSON。`with_key` 为真且是官方 CF 域名时挂上 key。
async fn get_json(
    client: &reqwest::Client,
    url: &str,
    params: &[(&str, String)],
    with_key: bool,
) -> Result<Value, AppError> {
    let mut request = client.get(url).query(params);
    if with_key
        && url.starts_with("https://api.curseforge.com")
        && let Some(key) = net::curseforge_key()
    {
        request = request.header("x-api-key", key);
    }
    let resp = request
        .send()
        .await
        .map_err(|err| AppError::Http(format!("请求 {url} 失败：{err}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "{url} 返回了 HTTP {}",
            resp.status()
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|err| AppError::Http(format!("{url} 返回的不是合法 JSON：{err}")))
}

/// 按顺序试一串候选地址，第一个成功的就用。
async fn get_json_any(
    client: &reqwest::Client,
    targets: &[(String, bool)],
    params: &[(&str, String)],
) -> Result<Value, AppError> {
    let mut last_error: Option<AppError> = None;
    for (url, with_key) in targets {
        match get_json(client, url, params, *with_key).await {
            Ok(value) => return Ok(value),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| AppError::Http("没有可用的下载源".to_string())))
}

/// CurseForge 候选：镜像排前面（没 key 也能用），官方有 key 时放最后兜底。
fn cf_targets(path: &str) -> Vec<(String, bool)> {
    let mut mirrors: Vec<(String, bool)> = Vec::new();
    let mut official: Vec<(String, bool)> = Vec::new();
    for (index, url) in net::curseforge_api_candidates(path).into_iter().enumerate() {
        if index == 0 {
            if net::curseforge_key().is_some() {
                official.push((url, true));
            }
        } else {
            mirrors.push((url, false));
        }
    }
    mirrors.extend(official);
    mirrors
}

/// Modrinth 候选：官方在前，镜像兜底。
fn modrinth_targets(path: &str) -> Vec<(String, bool)> {
    vec![
        (format!("{MODRINTH_API}{path}"), false),
        (format!("{MODRINTH_MIRROR}{path}"), false),
    ]
}

// ---------------------------------------------------------------------------
// 搜索
// ---------------------------------------------------------------------------

/// 搜索项目：`source` 为 `curseforge` 或 `modrinth`，`kind` 为资源类型。
pub async fn search(
    source: &str,
    kind: &str,
    query: &str,
    page: u32,
) -> Result<Vec<MarketProject>, AppError> {
    let client = http_client()?;
    let keyword = query.trim();
    match source {
        "modrinth" => search_modrinth(&client, kind, keyword, page).await,
        _ => search_curseforge(&client, kind, keyword, page).await,
    }
}

async fn search_curseforge(
    client: &reqwest::Client,
    kind: &str,
    keyword: &str,
    page: u32,
) -> Result<Vec<MarketProject>, AppError> {
    let mut params = vec![
        ("gameId", CF_GAME_ID.to_string()),
        ("classId", cf_class_id(kind).to_string()),
        ("sortField", "6".to_string()),
        ("sortOrder", "desc".to_string()),
        ("pageSize", PAGE_SIZE.to_string()),
        ("index", (page.saturating_mul(PAGE_SIZE)).to_string()),
    ];
    // 空关键词别传给 CF，不然它会把空串当筛选条件。
    if !keyword.is_empty() {
        params.push(("searchFilter", keyword.to_string()));
    }
    let value = get_json_any(client, &cf_targets("/v1/mods/search"), &params).await?;
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| parse_cf_project(item, kind))
        .collect())
}

async fn search_modrinth(
    client: &reqwest::Client,
    kind: &str,
    keyword: &str,
    page: u32,
) -> Result<Vec<MarketProject>, AppError> {
    let facets = format!("[[\"project_type:{}\"]]", mr_project_type(kind));
    let params = vec![
        ("query", keyword.to_string()),
        ("facets", facets),
        ("limit", PAGE_SIZE.to_string()),
        ("offset", (page.saturating_mul(PAGE_SIZE)).to_string()),
        ("index", "downloads".to_string()),
    ];
    let value = get_json_any(client, &modrinth_targets("/search"), &params).await?;
    let hits = value
        .get("hits")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(hits
        .iter()
        .filter_map(|item| parse_mr_project(item, kind))
        .collect())
}

fn parse_cf_project(item: &Value, kind: &str) -> Option<MarketProject> {
    let id = item.get("id").and_then(Value::as_i64)?.to_string();
    Some(MarketProject {
        source: "curseforge".to_string(),
        id,
        slug: text(item.get("slug")),
        name: {
            let name = text(item.get("name"));
            if name.is_empty() { "未命名".to_string() } else { name }
        },
        summary: text(item.get("summary")),
        icon: item
            .get("logo")
            .and_then(|logo| logo.get("thumbnailUrl"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|url| !url.is_empty()),
        downloads: item
            .get("downloadCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        author: item
            .get("authors")
            .and_then(Value::as_array)
            .and_then(|authors| authors.first())
            .map(|author| text(author.get("name")))
            .unwrap_or_default(),
        kind: kind.to_string(),
    })
}

fn parse_mr_project(item: &Value, kind: &str) -> Option<MarketProject> {
    Some(MarketProject {
        source: "modrinth".to_string(),
        id: text(item.get("project_id")),
        slug: text(item.get("slug")),
        name: {
            let name = text(item.get("title"));
            if name.is_empty() { "未命名".to_string() } else { name }
        },
        summary: text(item.get("description")),
        icon: item
            .get("icon_url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|url| !url.is_empty()),
        downloads: item.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        author: text(item.get("author")),
        kind: kind.to_string(),
    })
}

// ---------------------------------------------------------------------------
// 文件列表
// ---------------------------------------------------------------------------

/// 列某个项目下的文件；`game_version` / `loader` 传空串表示不筛。
pub async fn files(
    source: &str,
    project_id: &str,
    kind: &str,
    game_version: &str,
    loader: &str,
) -> Result<Vec<MarketFile>, AppError> {
    let client = http_client()?;
    let game_version = game_version.trim();
    let loader = loader.trim();
    match source {
        "modrinth" => files_modrinth(&client, project_id, kind, game_version, loader).await,
        _ => files_curseforge(&client, project_id, kind, game_version, loader).await,
    }
}

async fn files_curseforge(
    client: &reqwest::Client,
    project_id: &str,
    kind: &str,
    game_version: &str,
    loader: &str,
) -> Result<Vec<MarketFile>, AppError> {
    let mut params = vec![("pageSize", PAGE_SIZE.to_string())];
    if !game_version.is_empty() {
        params.push(("gameVersion", game_version.to_string()));
    }
    if let Some(loader_type) = cf_loader_type(loader) {
        params.push(("modLoaderType", loader_type.to_string()));
    }
    let path = format!("/v1/mods/{project_id}/files");
    let value = get_json_any(client, &cf_targets(&path), &params).await?;
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| parse_cf_file(item, kind))
        .collect())
}

fn parse_cf_file(item: &Value, kind: &str) -> Option<MarketFile> {
    let name = text(item.get("fileName"));
    if name.is_empty() {
        return None;
    }
    let mut game_versions: Vec<String> = Vec::new();
    let mut loaders: Vec<String> = Vec::new();
    if let Some(list) = item.get("gameVersions").and_then(Value::as_array) {
        for entry in list {
            let raw = entry.as_str().unwrap_or_default().trim();
            if raw.is_empty() {
                continue;
            }
            if CF_LOADER_NAMES
                .iter()
                .any(|known| known.eq_ignore_ascii_case(raw))
            {
                loaders.push(raw.to_string());
            } else {
                game_versions.push(raw.to_string());
            }
        }
    }
    // `downloadUrl` 有时是 null（作者禁用了第三方分发），那就按 fileId 拼公开 CDN。
    let url = item
        .get("downloadUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|url| !url.is_empty())
        .or_else(|| {
            let file_id = item.get("id").and_then(Value::as_i64)?;
            net::curseforge_cdn_url(file_id, &name)
        })?;
    Some(MarketFile {
        version: {
            let display = text(item.get("displayName"));
            if display.is_empty() { name.clone() } else { display }
        },
        name,
        game_versions,
        loaders,
        url,
        size: item.get("fileLength").and_then(Value::as_u64),
        sha1: item
            .get("hashes")
            .and_then(Value::as_array)
            .and_then(|hashes| {
                hashes
                    .iter()
                    .find(|hash| hash.get("algo").and_then(Value::as_u64) == Some(1))
                    .and_then(|hash| hash.get("value"))
                    .and_then(Value::as_str)
            })
            .map(str::to_string),
        folder: cf_file_folder(item, kind),
    })
}

/// CF 文件该放哪个目录：优先看 `modules`，没有再按资源类型猜。
fn cf_file_folder(item: &Value, kind: &str) -> String {
    if let Some(modules) = item.get("modules").and_then(Value::as_array) {
        let has = |name: &str| {
            modules
                .iter()
                .any(|module| module.get("name").and_then(Value::as_str) == Some(name))
        };
        if has("mods") {
            return "mods".to_string();
        }
        if has("resourcepacks") {
            return "resourcepacks".to_string();
        }
        if has("shaderpacks") {
            return "shaderpacks".to_string();
        }
        if has("worldtemplate") || has("datapacks") {
            return default_folder("datapack").to_string();
        }
    }
    default_folder(kind).to_string()
}

async fn files_modrinth(
    client: &reqwest::Client,
    project_id: &str,
    kind: &str,
    game_version: &str,
    loader: &str,
) -> Result<Vec<MarketFile>, AppError> {
    let mut params: Vec<(&str, String)> = Vec::new();
    if !game_version.is_empty() {
        params.push(("game_versions", format!("[\"{game_version}\"]")));
    }
    if !loader.is_empty() {
        params.push(("loaders", format!("[\"{}\"]", loader.to_ascii_lowercase())));
    }
    let path = format!("/project/{project_id}/version");
    let value = get_json_any(client, &modrinth_targets(&path), &params).await?;
    let versions = value.as_array().cloned().unwrap_or_default();
    let mut out: Vec<MarketFile> = Vec::new();
    for version in &versions {
        if let Some(file) = parse_mr_file(version, kind) {
            out.push(file);
        }
    }
    Ok(out)
}

fn parse_mr_file(version: &Value, kind: &str) -> Option<MarketFile> {
    let files = version.get("files").and_then(Value::as_array)?;
    // 有 primary 就用 primary，不然取第一个。
    let primary = files
        .iter()
        .find(|file| file.get("primary").and_then(Value::as_bool) == Some(true))
        .or_else(|| files.first())?;
    let name = text(primary.get("filename"));
    let url = text(primary.get("url"));
    if name.is_empty() || url.is_empty() {
        return None;
    }
    Some(MarketFile {
        version: {
            let number = text(version.get("version_number"));
            if number.is_empty() { name.clone() } else { number }
        },
        name,
        game_versions: string_list(version.get("game_versions")),
        loaders: string_list(version.get("loaders")),
        url,
        size: primary.get("size").and_then(Value::as_u64),
        sha1: primary
            .get("hashes")
            .and_then(|hashes| hashes.get("sha1"))
            .and_then(Value::as_str)
            .map(str::to_string),
        folder: default_folder(kind).to_string(),
    })
}

// ---------------------------------------------------------------------------
// 安装
// ---------------------------------------------------------------------------

/// 把某个文件装进实例（模组 / 资源包 / 光影包）。
pub async fn install_file(
    instance_id: &str,
    file: MarketFile,
    progress: ProgressFn<'_>,
) -> Result<MarketInstallReport, AppError> {
    let instance = instance::get_instance(instance_id)?;
    let folder = if file.folder.trim().is_empty() {
        "mods".to_string()
    } else {
        file.folder.trim().to_string()
    };
    let dir = PathBuf::from(&instance.game_dir).join(&folder);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(safe_name(&file.name));
    let job = FileJob {
        urls: download_candidates(&file.url),
        dest: dest.clone(),
        size: file.size,
        sha1: file.sha1.clone(),
    };
    let summary = download::download_jobs(vec![job], &format!("安装 {}", file.name), progress).await?;
    if !summary.failed.is_empty() {
        return Err(AppError::Http(format!(
            "{} 下载失败：{}",
            file.name,
            summary.failed.join("；")
        )));
    }
    Ok(MarketInstallReport {
        file_name: file.name,
        folder,
        path: dest.display().to_string(),
        skipped: summary.skipped > 0,
    })
}

/// 下载整合包并导入成新实例。
pub async fn install_modpack(
    file: MarketFile,
    progress: ProgressFn<'_>,
) -> Result<instance::modpack::ImportReport, AppError> {
    let dir = std::env::temp_dir().join("nightspark-modpacks");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(safe_name(&file.name));
    let job = FileJob {
        urls: download_candidates(&file.url),
        dest: dest.clone(),
        size: file.size,
        sha1: file.sha1.clone(),
    };
    let summary = download::download_jobs(
        vec![job],
        &format!("下载整合包 {}", file.name),
        progress,
    )
    .await?;
    if !summary.failed.is_empty() {
        return Err(AppError::Http(format!(
            "{} 下载失败：{}",
            file.name,
            summary.failed.join("；")
        )));
    }
    let path = dest.display().to_string();
    instance::import_modpack(&path)
}

/// 候选下载地址：原始直链在前，镜像替换在后。
fn download_candidates(url: &str) -> Vec<String> {
    let mut urls = vec![url.to_string()];
    let mirror = net::mirror_download(url);
    if mirror != url {
        urls.push(mirror);
    }
    urls
}

/// 只取文件名末段，挡掉 `../` 之类的目录穿越。
fn safe_name(name: &str) -> String {
    let last = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_string();
    if last.is_empty() || last == "." || last == ".." {
        "download.bin".to_string()
    } else {
        last
    }
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 资源类型要映射到各自的 classId，别一锅端。
    #[test]
    fn probe_class_ids() {
        assert_eq!(cf_class_id("mod"), 6);
        assert_eq!(cf_class_id("modpack"), 4471);
        assert_eq!(cf_class_id("resourcepack"), 12);
        assert_eq!(cf_class_id("shader"), 6552);
        assert_ne!(cf_class_id("mod"), cf_class_id("modpack"));
    }

    /// 加载器名要能换成 CF 的枚举值，不认识的返回 None。
    #[test]
    fn probe_loader_type() {
        assert_eq!(cf_loader_type("Forge"), Some(1));
        assert_eq!(cf_loader_type("fabric"), Some(4));
        assert_eq!(cf_loader_type("neoforge"), Some(6));
        assert_eq!(cf_loader_type("paper"), None);
    }

    /// 文件名要能挡住目录穿越。
    #[test]
    fn probe_safe_name() {
        assert_eq!(safe_name("../../evil.jar"), "evil.jar");
        assert_eq!(safe_name("C:\\Windows\\a.jar"), "a.jar");
        assert_eq!(safe_name("  "), "download.bin");
    }

    /// CF 的 modules 决定落点目录，缺信息时按资源类型兜底。
    #[test]
    fn probe_folder() {
        let mod_file = serde_json::json!({"modules": [{"name": "META-INF"}, {"name": "mods"}]});
        assert_eq!(cf_file_folder(&mod_file, "mod"), "mods");
        let bare = serde_json::json!({});
        assert_eq!(cf_file_folder(&bare, "shader"), "shaderpacks");
        assert_eq!(cf_file_folder(&bare, "resourcepack"), "resourcepacks");
    }

    /// Modrinth 版本要挑 primary 文件，并带上 sha1 供校验。
    #[test]
    fn probe_mr_file() {
        let version = serde_json::json!({
            "version_number": "1.2.3",
            "game_versions": ["1.20.1"],
            "loaders": ["forge"],
            "files": [
                {"filename": "extra.jar", "url": "https://cdn/x.jar", "primary": false},
                {"filename": "main.jar", "url": "https://cdn/main.jar", "primary": true,
                 "hashes": {"sha1": "abc"}, "size": 10}
            ]
        });
        let file = parse_mr_file(&version, "mod").unwrap();
        assert_eq!(file.name, "main.jar");
        assert_eq!(file.sha1.as_deref(), Some("abc"));
        assert_eq!(file.folder, "mods");
    }

    /// 联网联调（默认 ignore）：cargo test live_market -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_market_roundtrip() {
        for source in ["curseforge", "modrinth"] {
            let mods = search(source, "mod", "jei", 0).await.expect("模组搜索失败");
            assert!(!mods.is_empty(), "{source} 模组搜索没返回内容");
            let packs = search(source, "modpack", "", 0)
                .await
                .expect("整合包搜索失败");
            assert!(!packs.is_empty(), "{source} 整合包搜索没返回内容");
            let target = &mods[0];
            let packed = &packs[0];
            println!(
                "[{source}] 模组 {} ({}) / 整合包 {} ({})",
                target.name, target.downloads, packed.name, packed.downloads
            );
            for (kind, project) in [("mod", target), ("modpack", packed)] {
                let files = files(source, &project.id, kind, "", "")
                    .await
                    .expect("文件列表失败");
                assert!(!files.is_empty(), "{source}/{kind} 文件列表为空");
                let first = &files[0];
                assert!(first.url.starts_with("http"), "{source} 直链异常");
                println!(
                    "[{source}/{kind}] {} -> {} ({}) sha1={:?}",
                    first.version, first.name, first.folder, first.sha1.is_some()
                );
            }
        }
    }
}
