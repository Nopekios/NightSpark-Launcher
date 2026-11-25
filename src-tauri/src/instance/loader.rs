//! 加载器安装：Fabric / Quilt 走官方 profile json，Forge / NeoForge 走官方 installer。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::config;
use crate::download::{self, FileJob, ProgressFn};
use crate::error::AppError;
use crate::instance;
use crate::net;

/// installer 内部条目名，用来确认下到的确实是加载器安装包。
const INSTALLER_PROFILE: &str = "install_profile.json";
/// 装 installer 用的临时子目录（相对实例的游戏目录）。
const INSTALL_TMP_DIR: &str = ".nightspark/loader-install";
/// Forge / NeoForge 的 installer 断网类错误最多重试几次（库已下好，重跑很便宜）。
const INSTALLER_ATTEMPTS: u32 = 3;
/// 给 installer 看的 profile 文件内容：只求它能认出这是个合法启动器目录。
const LAUNCHER_PROFILE_BODY: &str =
    "{\"profiles\":{},\"settings\":{},\"version\":3}";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoaderKind {
    Forge,
    NeoForge,
    Fabric,
    Quilt,
}

impl LoaderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
            LoaderKind::Fabric => "fabric",
            LoaderKind::Quilt => "quilt",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "forge" => Some(LoaderKind::Forge),
            "neoforge" | "neo" => Some(LoaderKind::NeoForge),
            "fabric" | "fabric-loader" => Some(LoaderKind::Fabric),
            "quilt" | "quilt-loader" => Some(LoaderKind::Quilt),
            _ => None,
        }
    }
}

/// 解析 `forge-47.2.0` / `neoforge` / `fabric 0.16.9` 这类写法。
///
/// CurseForge 的 `modLoaders` 有时写成 `forge-1.20.1-47.2.0`（带游戏版本），
/// 这里只取最后一段当加载器版本号。
pub fn parse_loader_spec(spec: &str) -> Option<(LoaderKind, Option<String>)> {
    let text = spec.trim();
    if text.is_empty() {
        return None;
    }
    let (head, tail) = match text.split_once('-') {
        Some((head, tail)) => (head, Some(tail)),
        None => match text.split_once(' ') {
            Some((head, tail)) => (head, Some(tail)),
            None => (text, None),
        },
    };
    let kind = LoaderKind::parse(head)?;
    let version = tail.and_then(|rest| {
        rest.rsplit('-')
            .next()
            .map(str::trim)
            .filter(|value| {
                !value.is_empty()
                    && value
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '_')
            })
            .map(str::to_string)
    });
    Some((kind, version))
}

#[derive(Serialize, Clone, Debug)]
pub struct LoaderReport {
    pub loader: String,
    pub loader_version: String,
    pub mc_version: String,
    pub target_version: String,
    pub version_json: String,
    pub downloaded: usize,
    pub skipped: usize,
    pub messages: Vec<String>,
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent("NightSpark/0.1")
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| AppError::Http(format!("创建 HTTP 客户端失败：{err}")))
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value, AppError> {
    let resp = client
        .get(url)
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

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Http(format!("请求 {url} 失败：{err}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "{url} 返回了 HTTP {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|err| AppError::Http(format!("读取 {url} 响应失败：{err}")))
}

/// 把 maven 坐标转成仓库里的相对路径，`group:artifact:version` -> `group/artifact/version/artifact-version.jar`。
fn maven_relative(name: &str) -> Option<String> {
    let mut parts = name.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    // 第 4 段是 classifier（natives 之类），第 5 段才是扩展名。
    let classifier = parts.next();
    let extension = parts.next().unwrap_or("jar");
    let file = match classifier {
        Some(classifier) if !classifier.is_empty() => {
            format!("{artifact}-{version}-{classifier}.{extension}")
        }
        _ => format!("{artifact}-{version}.{extension}"),
    };
    Some(format!(
        "{}/{}/{}/{}",
        group.replace('.', "/"),
        artifact,
        version,
        file
    ))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<usize, AppError> {
    let mut copied = 0usize;
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copied += copy_tree(&from, &to)?;
            continue;
        }
        let same = match (std::fs::metadata(&from), std::fs::metadata(&to)) {
            (Ok(a), Ok(b)) => a.len() == b.len(),
            _ => false,
        };
        if same {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&from, &to)?;
        copied += 1;
    }
    Ok(copied)
}

/// 把一份版本目录同时摆到「实例目录」和「共享版本目录」，启动器两边都能找到。
fn publish_version(
    game_dir: &Path,
    version_id: &str,
    source_dir: &Path,
    messages: &mut Vec<String>,
) -> Result<PathBuf, AppError> {
    let local_target = game_dir.join("versions").join(version_id);
    if source_dir != local_target {
        copy_tree(source_dir, &local_target)?;
    }
    let json_path = local_target.join(format!("{version_id}.json"));
    if !json_path.is_file() {
        return Err(AppError::Config(format!(
            "安装完了却没在 {} 里找到 {version_id}.json",
            local_target.display()
        )));
    }

    // 共享版本目录是启动时读元数据的地方，同步一份过去。
    match download::versions_dir() {
        Ok(shared) => {
            let shared_target = shared.join(version_id);
            if shared_target != local_target {
                if let Err(err) = copy_tree(&local_target, &shared_target) {
                    messages.push(format!("同步到共享版本目录失败：{err}"));
                }
            }
        }
        Err(err) => messages.push(format!("拿不到共享版本目录：{err}")),
    }
    Ok(json_path)
}

/// 把原版 client jar 补进加载器版本目录，加载器本身不产出这个 jar。
fn ensure_client_jar(game_dir: &Path, version_id: &str, mc_version: &str, messages: &mut Vec<String>) {
    let target = game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.jar"));
    if target.is_file() {
        return;
    }
    let Ok(shared) = download::versions_dir() else {
        return;
    };
    let source = shared
        .join(mc_version)
        .join(format!("{mc_version}.jar"));
    if !source.is_file() {
        messages.push(format!(
            "共享目录里没有原版 {mc_version} 的 client jar，启动前得先把 {mc_version} 装好"
        ));
        return;
    }
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(&source, &target) {
        Ok(_) => {
            // 共享版本目录里也补一份，启动器两个位置都可能读。
            if let Ok(shared) = download::versions_dir() {
                let mirror = shared.join(version_id).join(format!("{version_id}.jar"));
                if let Some(parent) = mirror.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&target, &mirror);
            }
        }
        Err(err) => messages.push(format!("复制 client jar 失败：{err}")),
    }
}

/// 挑一个能跑 installer 的 Java：优先入参，其次用户配置，最后挑本机主版本最高的。
fn pick_java(explicit: Option<&str>) -> Result<PathBuf, AppError> {
    if let Some(path) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let saved = config::load_config()?;
    if let Some(path) = saved
        .java_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    let list = config::detect_java()?;
    let mut best: Option<(u32, String)> = None;
    for java in list {
        let major = java
            .version
            .split('.')
            .next()
            .and_then(|head| head.parse::<u32>().ok())
            .unwrap_or(0);
        // 版本串常写成 1.8.0_392，主版本要往后退一位。
        let major = if major == 1 {
            java.version
                .split('.')
                .nth(1)
                .and_then(|next| next.parse::<u32>().ok())
                .unwrap_or(major)
        } else {
            major
        };
        if best.as_ref().map(|(known, _)| major > *known).unwrap_or(true) {
            best = Some((major, java.path));
        }
    }
    match best {
        Some((_, path)) => Ok(PathBuf::from(path)),
        None => Err(AppError::NotFound(
            "本机没找到可用的 Java，Forge / NeoForge 的安装器必须靠 Java 才能跑".to_string(),
        )),
    }
}

/// 查 Forge 对应游戏版本的推荐版本（先推荐、后最新）。
async fn resolve_forge_version(client: &reqwest::Client, mc_version: &str) -> Result<String, AppError> {
    let mirrors = [
        "https://bmclapi2.bangbang93.com/forge/promotions_slim.json",
        "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
    ];
    let mut last_error: Option<String> = None;
    for url in mirrors {
        match fetch_json(client, url).await {
            Ok(value) => {
                let Some(promos) = value.get("promos") else {
                    continue;
                };
                for key in [
                    format!("{mc_version}-recommended"),
                    format!("{mc_version}-latest"),
                ] {
                    if let Some(version) = promos.get(&key).and_then(Value::as_str) {
                        return Ok(version.to_string());
                    }
                }
                return Err(AppError::NotFound(format!(
                    "Forge 的版本表里没有 {mc_version}，可能是还没支持这个游戏版本"
                )));
            }
            Err(err) => last_error = Some(err.to_string()),
        }
    }
    Err(AppError::Http(format!(
        "拿不到 Forge 版本表：{}",
        last_error.unwrap_or_else(|| "所有镜像都没响应".to_string())
    )))
}

/// 查 NeoForge 对应游戏版本的最新版本号。
///
/// 1.20.1 的 NeoForge 还挂在 `net/neoforged/forge` 下面，版本号写成 `1.20.1-47.1.x`；
/// 1.20.2 之后才改成 `net/neoforged/neoforge` + `21.1.x` 这种按游戏版本编号的形式。
async fn resolve_neoforge_version(
    client: &reqwest::Client,
    mc_version: &str,
) -> Result<String, AppError> {
    let url = if mc_version == "1.20.1" {
        format!("{}/net/neoforged/forge/maven-metadata.xml", net::NEOFORGE_MAVEN)
    } else {
        format!("{}/net/neoforged/neoforge/maven-metadata.xml", net::NEOFORGE_MAVEN)
    };
    let xml = fetch_text(client, &url).await?;
    let mut picked: Option<(Vec<u32>, String)> = None;
    let prefix = if mc_version == "1.20.1" {
        format!("{mc_version}-")
    } else {
        // 20.2 -> 20.2.，21.1 -> 21.1.
        let mut parts = mc_version.split('.');
        let _ = parts.next();
        let rest: Vec<&str> = parts.collect();
        format!("{}.", rest.join("."))
    };
    for chunk in xml.split("<version>").skip(1) {
        let Some(raw) = chunk.split("</version>").next() else {
            continue;
        };
        let version = raw.trim();
        if !version.starts_with(&prefix) {
            continue;
        }
        let numbers: Vec<u32> = version
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|piece| !piece.is_empty())
            .filter_map(|piece| piece.parse::<u32>().ok())
            .collect();
        if picked
            .as_ref()
            .map(|(known, _)| numbers > *known)
            .unwrap_or(true)
        {
            picked = Some((numbers, version.to_string()));
        }
    }
    picked
        .map(|(_, version)| version)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "NeoForge 的版本表里没有和 {mc_version} 对上的版本"
            ))
        })
}

fn neoforge_coords(mc_version: &str, loader_version: &str) -> (String, String) {
    if mc_version == "1.20.1" {
        (
            "net/neoforged/forge".to_string(),
            format!("{mc_version}-{loader_version}"),
        )
    } else {
        (
            "net/neoforged/neoforge".to_string(),
            loader_version.to_string(),
        )
    }
}

fn installer_urls(kind: LoaderKind, mc_version: &str, loader_version: &str) -> Vec<String> {
    match kind {
        LoaderKind::Forge => {
            let artifact_version = format!("{mc_version}-{loader_version}");
            let path = format!(
                "net/minecraftforge/forge/{artifact_version}/forge-{artifact_version}-installer.jar"
            );
            vec![
                format!("{}/{}", net::MCFORGE_MAVEN_MIRROR, path),
                format!("{}/{}", net::MCFORGE_MAVEN, path),
            ]
        }
        LoaderKind::NeoForge => {
            let (group_path, artifact_version) = neoforge_coords(mc_version, loader_version);
            let artifact = if mc_version == "1.20.1" { "forge" } else { "neoforge" };
            let path = format!(
                "{}/{}/{artifact}-{artifact_version}-installer.jar",
                group_path, artifact_version
            );
            vec![format!("{}/{}", net::NEOFORGE_MAVEN, path)]
        }
        _ => Vec::new(),
    }
}

fn profile_urls(kind: LoaderKind, mc_version: &str, loader_version: &str) -> Vec<String> {
    match kind {
        LoaderKind::Fabric => vec![
            format!(
                "{}/v2/versions/loader/{mc_version}/{loader_version}/profile/json",
                net::FABRIC_META
            ),
            format!(
                "https://bmclapi2.bangbang93.com/fabric-meta/v2/versions/loader/{mc_version}/{loader_version}/profile/json"
            ),
        ],
        LoaderKind::Quilt => vec![format!(
            "{}/v3/versions/loader/{mc_version}/{loader_version}/profile/json",
            net::QUILT_META
        )],
        _ => Vec::new(),
    }
}

async fn resolve_fabric_version(
    client: &reqwest::Client,
    kind: LoaderKind,
    mc_version: &str,
) -> Result<String, AppError> {
    let url = match kind {
        LoaderKind::Fabric => format!("{}/v2/versions/loader/{mc_version}", net::FABRIC_META),
        LoaderKind::Quilt => format!("{}/v3/versions/loader/{mc_version}", net::QUILT_META),
        _ => {
            return Err(AppError::Internal(format!(
                "{} 不是 Fabric 系加载器",
                kind.as_str()
            )))
        }
    };
    let value = fetch_json(client, &url).await?;
    let list = value
        .as_array()
        .ok_or_else(|| AppError::Config(format!("{url} 返回的格式不对")))?;
    for item in list {
        if let Some(version) = item
            .get("loader")
            .and_then(|loader| loader.get("version"))
            .and_then(Value::as_str)
        {
            return Ok(version.to_string());
        }
    }
    Err(AppError::NotFound(format!(
        "{mc_version} 找不到可用的 {} 版本",
        kind.as_str()
    )))
}

async fn install_profile_loader(
    kind: LoaderKind,
    instance: &instance::Instance,
    mc_version: &str,
    loader_version: &str,
    progress: ProgressFn<'_>,
) -> Result<LoaderReport, AppError> {
    let game_dir = PathBuf::from(&instance.game_dir);
    progress("profile", 0, 1);
    let client = http_client()?;
    let mut profile: Option<Value> = None;
    let mut last_error: Option<String> = None;
    for url in profile_urls(kind, mc_version, loader_version) {
        match fetch_json(&client, &url).await {
            Ok(value) => {
                profile = Some(value);
                break;
            }
            Err(err) => last_error = Some(err.to_string()),
        }
    }
    let profile = profile.ok_or_else(|| {
        AppError::Http(format!(
            "拿不到 {} 的 profile：{}",
            kind.as_str(),
            last_error.unwrap_or_else(|| "所有镜像都没响应".to_string())
        ))
    })?;

    let target_version = profile
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AppError::Config("profile 里没有 id 字段".to_string()))?;

    let mut messages: Vec<String> = Vec::new();
    let local_dir = game_dir.join("versions").join(&target_version);
    std::fs::create_dir_all(&local_dir)?;
    let json_path = local_dir.join(format!("{target_version}.json"));
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&profile).map_err(|err| AppError::Config(err.to_string()))?,
    )?;
    progress("profile", 1, 1);

    let mut jobs: Vec<FileJob> = Vec::new();
    let libraries_dir = download::libraries_dir()?;
    if let Some(libraries) = profile.get("libraries").and_then(Value::as_array) {
        for library in libraries {
            let Some(name) = library.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(relative) = maven_relative(name) else {
                messages.push(format!("跳过无法识别的库坐标：{name}"));
                continue;
            };
            let base = library
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://maven.fabricmc.net/");
            let url = format!("{}{}", base.trim_end_matches('/'), format!("/{relative}"));
            jobs.push(FileJob {
                urls: vec![url],
                dest: libraries_dir.join(&relative),
                size: None,
                sha1: None,
            });
        }
    }
    let summary = download::download_jobs(jobs, "loader-libraries", progress).await?;
    messages.extend(summary.failed.clone());

    ensure_client_jar(&game_dir, &target_version, mc_version, &mut messages);
    publish_version(&game_dir, &target_version, &local_dir, &mut messages)?;

    Ok(LoaderReport {
        loader: kind.as_str().to_string(),
        loader_version: loader_version.to_string(),
        mc_version: mc_version.to_string(),
        target_version,
        version_json: json_path.to_string_lossy().into_owned(),
        downloaded: summary.downloaded,
        skipped: summary.skipped,
        messages,
    })
}

/// 从 installer 包里直接解出加载器：老版本 Forge 没有 processors，装法就是
/// 把 universal 摆进 libraries、把 versionInfo 写成版本 json。
/// 返回 Ok(None) 表示这个安装器得靠真跑 Java 才能装（1.13+ 带 processors 的）。
#[allow(clippy::too_many_arguments)]
async fn install_from_installer_profile(
    kind: LoaderKind,
    game_dir: &Path,
    installer_path: &Path,
    mc_version: &str,
    loader_version: &str,
    base_downloaded: usize,
    base_skipped: usize,
    messages: &mut Vec<String>,
    progress: ProgressFn<'_>,
) -> Result<Option<LoaderReport>, AppError> {
    let file = std::fs::File::open(installer_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Config(format!("安装包不是合法 jar：{err}")))?;
    let profile_text = read_zip_text(&mut zip, INSTALLER_PROFILE)
        .ok_or_else(|| AppError::Config("安装包里没有 install_profile.json".to_string()))?;
    let profile: Value = serde_json::from_str(&profile_text)
        .map_err(|err| AppError::Config(format!("install_profile.json 解析失败：{err}")))?;

    // 1.13 起安装器要靠 processors 打补丁，解包不够，交给真安装器。
    let has_processors = profile
        .get("processors")
        .and_then(Value::as_array)
        .map(|list| !list.is_empty())
        .unwrap_or(false);
    if has_processors {
        return Ok(None);
    }
    let mut version_info = match profile.get("versionInfo").cloned() {
        Some(value) if value.is_object() => value,
        _ => match read_zip_text(&mut zip, "version.json")
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            Some(value) => value,
            None => return Ok(None),
        },
    };
    if version_info.get("inheritsFrom").is_none() {
        version_info["inheritsFrom"] = Value::String(mc_version.to_string());
    }
    let target_version = version_info
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{mc_version}-{}-{loader_version}", kind.as_str()));
    version_info["id"] = Value::String(target_version.clone());

    let libraries_dir = download::libraries_dir()?;
    let install = profile.get("install").cloned().unwrap_or(Value::Null);
    let forge_coords = install
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    if let (Some(coords), Some(file_path)) = (
        install.get("path").and_then(Value::as_str).map(str::trim),
        install.get("filePath").and_then(Value::as_str).map(str::trim),
    ) {
        if let Some(relative) = maven_relative(coords) {
            let dest = libraries_dir.join(&relative);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut entry = zip.by_name(file_path).map_err(|err| {
                AppError::Config(format!("安装包里找不到 {file_path}：{err}"))
            })?;
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
            messages.push(format!("解出 {file_path}"));
        }
    }

    let mut jobs: Vec<FileJob> = Vec::new();
    if let Some(libraries) = version_info.get("libraries").and_then(Value::as_array) {
        for library in libraries {
            let Some(name) = library.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !forge_coords.is_empty() && name.trim() == forge_coords {
                continue;
            }
            let Some(relative) = maven_relative(name) else {
                messages.push(format!("跳过无法识别的库坐标：{name}"));
                continue;
            };
            let dest = libraries_dir.join(&relative);
            if dest.is_file() {
                continue;
            }
            // forge maven 里缺的库（launchwrapper 之类）只在官方或镜像上，
            // 所以同一份库里给三个源，按顺序试。
            let base = library
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://maven.minecraftforge.net/")
                .replace("http://files.minecraftforge.net/maven", "https://maven.minecraftforge.net")
                .replace("http://", "https://");
            let mut urls = vec![format!("{}/{}", base.trim_end_matches('/'), relative)];
            for mirror in [
                "https://bmclapi2.bangbang93.com/maven",
                "https://libraries.minecraft.net",
            ] {
                let url = format!("{mirror}/{relative}");
                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
            jobs.push(FileJob {
                urls,
                dest,
                size: None,
                sha1: None,
            });
        }
    }
    let summary = if jobs.is_empty() {
        None
    } else {
        progress("loader-libraries", 0, jobs.len() as u64);
        Some(download::download_jobs(jobs, "loader-libraries", progress).await?)
    };

    let local_dir = game_dir.join("versions").join(&target_version);
    std::fs::create_dir_all(&local_dir)?;
    let json_path = local_dir.join(format!("{target_version}.json"));
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&version_info).map_err(|err| AppError::Config(err.to_string()))?,
    )?;
    ensure_client_jar(game_dir, &target_version, mc_version, messages);
    publish_version(game_dir, &target_version, &local_dir, messages)?;

    let (extra_downloaded, extra_skipped) = match summary {
        Some(summary) => {
            if !summary.failed.is_empty() {
                messages.push(format!(
                    "{} 个依赖库没下下来（{}），启动若报缺库再点一次安装",
                    summary.failed.len(),
                    summary.failed.join("；")
                ));
            }
            (summary.downloaded, summary.skipped)
        }
        None => (0, 0),
    };

    Ok(Some(LoaderReport {
        loader: kind.as_str().to_string(),
        loader_version: loader_version.to_string(),
        mc_version: mc_version.to_string(),
        target_version,
        version_json: json_path.to_string_lossy().into_owned(),
        downloaded: base_downloaded + extra_downloaded,
        skipped: base_skipped + extra_skipped,
        messages: messages.clone(),
    }))
}

fn read_zip_text<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<String> {
    use std::io::Read;
    let mut entry = zip.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

async fn install_binary_loader(
    kind: LoaderKind,
    instance: &instance::Instance,
    mc_version: &str,
    loader_version: &str,
    java_path: Option<&str>,
    progress: ProgressFn<'_>,
) -> Result<LoaderReport, AppError> {
    let game_dir = PathBuf::from(&instance.game_dir);
    let tmp_dir = game_dir.join(INSTALL_TMP_DIR);
    std::fs::create_dir_all(&tmp_dir)?;

    let installer_name = format!("{}-{}-installer.jar", kind.as_str(), loader_version);
    let installer_path = tmp_dir.join(&installer_name);
    let urls = installer_urls(kind, mc_version, loader_version);
    if urls.is_empty() {
        return Err(AppError::Internal(format!(
            "{} 没有可用的安装包地址",
            kind.as_str()
        )));
    }
    let summary = download::download_jobs(
        vec![FileJob {
            urls,
            dest: installer_path.clone(),
            size: None,
            sha1: None,
        }],
        "installer",
        progress,
    )
    .await?;
    if !summary.failed.is_empty() {
        return Err(AppError::Http(format!(
            "下载 {} 安装包失败：{}",
            kind.as_str(),
            summary.failed.join("；")
        )));
    }

    // 2) 确认它确实是安装包（有 install_profile.json）
    let file = std::fs::File::open(&installer_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Config(format!("安装包不是合法 jar：{err}")))?;
    let has_profile = (0..zip.len()).any(|index| {
        zip.by_index(index)
            .map(|entry| entry.name() == INSTALLER_PROFILE)
            .unwrap_or(false)
    });
    if !has_profile {
        return Err(AppError::Config(format!(
            "{} 里没有 {INSTALLER_PROFILE}，这不是加载器安装包",
            installer_path.display()
        )));
    }
    drop(zip);
    let mut messages: Vec<String> = Vec::new();

    // 2.5) 老版本 Forge（1.12.2 及更早）的 installer 不认 --installClient，
    // 这类安装器其实只是把 universal 解到 libraries 再写个版本 json，直接照做，省掉跑 Java。
    match install_from_installer_profile(
        kind,
        &game_dir,
        &installer_path,
        mc_version,
        loader_version,
        summary.downloaded,
        summary.skipped,
        &mut messages,
        progress,
    )
    .await
    {
        Ok(Some(report)) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(report);
        }
        Ok(None) => {}
        Err(err) => {
            let _ = err;
        }
    }

    // 3) 准备一个「看起来像官方启动器」的目录，installer 只认这个
    let work_dir = tmp_dir.join("work");
    std::fs::create_dir_all(work_dir.join("versions"))?;
    std::fs::write(
        work_dir.join("launcher_profiles.json"),
        LAUNCHER_PROFILE_BODY,
    )?;

    // installer 自己也会往 work/versions 里写一份原版版本，先记下装之前有哪些，
    // 后面挑产物时只认加载器自己的那一份，别把原版当成「装好的加载器」。
    let versions_before = list_version_dirs(&work_dir.join("versions"));

    // 4) 跑 installer。DNS 抖动、临时断流会让它整段退出（UnknownHostException 之类），
    //    但已下好的库能复用，歇一下重跑往往就过了，直接判死刑太亏。
    let java = pick_java(java_path)?;
    let mut last_code: Option<i32> = None;
    let mut last_tail = String::new();
    let mut succeeded = false;
    for attempt in 1..=INSTALLER_ATTEMPTS {
        progress("installer", 0, 1);
        let mut cmd = tokio::process::Command::new(&java);
        cmd.arg("-jar")
            .arg(&installer_path)
            .arg("--installClient")
            .arg(&work_dir)
            .current_dir(&tmp_dir);
        // 安装器是控制台程序，不加这个标志 Windows 会为它弹出一个黑框窗口。
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().await.map_err(|err| {
            AppError::Internal(format!("启动 Java ({}) 失败：{err}", java.display()))
        })?;
        progress("installer", 1, 1);
        if output.status.success() {
            succeeded = true;
            break;
        }
        last_code = output.status.code();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        last_tail = stdout
            .lines()
            .chain(stderr.lines())
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" / ");
        let hay = format!("{stdout}{stderr}").to_ascii_lowercase();
        let transient = [
            "unknownhostexception",
            "connectexception",
            "sockettimeout",
            "sslhandshake",
            "connection reset",
        ]
        .iter()
        .any(|key| hay.contains(key));
        if !transient || attempt == INSTALLER_ATTEMPTS {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3 * attempt as u64)).await;
    }
    if !succeeded {
        let log_path = PathBuf::from(format!("{}.log", installer_path.display()));
        let hint = if log_path.is_file() {
            format!("（详细日志：{}）", log_path.display())
        } else {
            String::new()
        };
        return Err(AppError::Internal(format!(
            "{} 安装器退出码 {:?}：{}{}",
            kind.as_str(),
            last_code,
            last_tail,
            hint
        )));
    }

    let versions_root = work_dir.join("versions");
    let (source_dir, target_version) =
        pick_installed_version(kind, mc_version, &versions_root, &versions_before).ok_or_else(|| {
            AppError::Config(format!(
                "{} 安装器跑完了，但 {} 里没有产出任何版本",
                kind.as_str(),
                versions_root.display()
            ))
        })?;

    let mut messages: Vec<String> = Vec::new();
    let json_path = publish_version(&game_dir, &target_version, &source_dir, &mut messages)?;
    let libraries_source = work_dir.join("libraries");
    if libraries_source.is_dir() {
        let libraries_dir = download::libraries_dir()?;
        match copy_tree(&libraries_source, &libraries_dir) {
            Ok(count) => {
                if count > 0 {
                    messages.push(format!("补齐了 {count} 个加载器依赖库"));
                }
            }
            Err(err) => messages.push(format!("合并 libraries 失败：{err}")),
        }
    }
    ensure_client_jar(&game_dir, &target_version, mc_version, &mut messages);
    // 临时目录留着只会占地儿，装完就清掉。
    let _ = std::fs::remove_dir_all(&tmp_dir);

    Ok(LoaderReport {
        loader: kind.as_str().to_string(),
        loader_version: loader_version.to_string(),
        mc_version: mc_version.to_string(),
        target_version,
        version_json: json_path.to_string_lossy().into_owned(),
        downloaded: summary.downloaded,
        skipped: summary.skipped,
        messages,
    })
}

/// 扫一遍 versions 目录下已有的版本名（只看带同名 json 的目录）。
fn list_version_dirs(root: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return names;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().join(format!("{name}.json")).is_file() {
            names.push(name);
        }
    }
    names
}

/// installer 会把原版版本（例如 `1.21.1`）也写进 work/versions，
/// 取产物时必须优先认加载器自己那份，否则会把原版 json 当安装结果，启动又变回原版。
fn pick_installed_version(
    kind: LoaderKind,
    mc_version: &str,
    root: &std::path::Path,
    before: &[String],
) -> Option<(PathBuf, String)> {
    let marker = kind.as_str().to_ascii_lowercase();
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().join(format!("{name}.json")).is_file() {
            continue;
        }
        entries.push((entry.path(), name));
    }
    let is_loader_name = |name: &str| {
        let lower = name.to_ascii_lowercase();
        ["forge", "fabric", "quilt"].iter().any(|key| lower.contains(key))
    };
    entries
        .iter()
        .find(|(_, name)| name.to_ascii_lowercase().contains(&marker))
        .or_else(|| entries.iter().find(|(_, name)| is_loader_name(name)))
        .or_else(|| {
            entries
                .iter()
                .find(|(_, name)| name != mc_version && !before.contains(name))
        })
        .or_else(|| entries.iter().find(|(_, name)| name != mc_version))
        .cloned()
}

/// `spec` 可以写 `forge`、`neoforge`、`fabric`，也可以带版本 `forge-47.2.0`；
/// `loader_version` 入参优先于 `spec` 里带的版本。
pub async fn install(
    instance_id: &str,
    spec: &str,
    loader_version: Option<String>,
    mc_version: Option<String>,
    java_path: Option<String>,
    progress: ProgressFn<'_>,
) -> Result<LoaderReport, AppError> {
    let instance = instance::get_instance(instance_id)?;
    // 用户没填（或填 auto）时，用整合包导入时记下的加载器要求。
    let trimmed = spec.trim();
    let pack_meta = if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        read_pack_meta(&instance.game_dir)
    } else {
        None
    };
    let spec_text = match &pack_meta {
        Some((loader, _)) if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") => {
            loader.clone()
        }
        _ => trimmed.to_string(),
    };
    if spec_text.is_empty() {
        return Err(AppError::Config(format!(
            "不知道该装哪个加载器，写上版本吧，比如 forge-47.2.0 或 fabric-0.15.11"
        )));
    }
    let (kind, parsed_version) = parse_loader_spec(&spec_text).ok_or_else(|| {
        AppError::Config(format!(
            "看不懂加载器写法「{spec_text}」，支持 forge / neoforge / fabric / quilt"
        ))
    })?;
    let mc_version = mc_version
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            pack_meta
                .as_ref()
                .map(|(_, mc)| mc.trim().to_string())
                .filter(|mc| !mc.is_empty())
        })
        .unwrap_or_else(|| instance.version.clone());
    if mc_version.is_empty() {
        return Err(AppError::Config(
            "不知道该给哪个游戏版本装加载器，请先选好实例版本".to_string(),
        ));
    }

    let client = http_client()?;
    let loader_version = match loader_version
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(parsed_version)
    {
        Some(version) => version,
        None => match kind {
            LoaderKind::Forge => resolve_forge_version(&client, &mc_version).await?,
            LoaderKind::NeoForge => resolve_neoforge_version(&client, &mc_version).await?,
            LoaderKind::Fabric | LoaderKind::Quilt => {
                resolve_fabric_version(&client, kind, &mc_version).await?
            }
        },
    };

    let report = match kind {
        LoaderKind::Fabric | LoaderKind::Quilt => {
            install_profile_loader(kind, &instance, &mc_version, &loader_version, progress).await?
        }
        LoaderKind::Forge | LoaderKind::NeoForge => {
            install_binary_loader(
                kind,
                &instance,
                &mc_version,
                &loader_version,
                java_path.as_deref(),
                progress,
            )
            .await?
        }
    };

    // 实例的 version 指向新装出来的加载器版本，启动时才会带上 mods 加载能力。
    let mut updated = instance;
    updated.version = report.target_version.clone();
    let game_dir = PathBuf::from(&updated.game_dir);
    if let Some(instance_dir) = game_dir.parent() {
        instance::write_instance(instance_dir, &updated)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 加载器写法要能吃下 CurseForge 的 `forge-47.2.0` 和「带游戏版本」的变体。
    #[test]
    fn probe_parse_loader_spec() {
        let (kind, version) = parse_loader_spec("forge-47.2.0").unwrap();
        assert_eq!(kind, LoaderKind::Forge);
        assert_eq!(version.as_deref(), Some("47.2.0"));

        let (kind, version) = parse_loader_spec("forge-1.20.1-47.2.0").unwrap();
        assert_eq!(kind, LoaderKind::Forge);
        assert_eq!(version.as_deref(), Some("47.2.0"));

        let (kind, version) = parse_loader_spec("neoforge").unwrap();
        assert_eq!(kind, LoaderKind::NeoForge);
        assert!(version.is_none());

        assert!(parse_loader_spec("optifine-1.0").is_none());
        assert!(parse_loader_spec("").is_none());
    }

    #[test]
    fn probe_maven_relative() {
        assert_eq!(
            maven_relative("net.fabricmc:fabric-loader:0.16.9").unwrap(),
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
        assert_eq!(
            maven_relative("org.lwjgl:lwjgl:3.3.1:natives-windows").unwrap(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );
        assert!(maven_relative("bad").is_none());
    }

    #[test]
    fn probe_installer_urls() {
        let urls = installer_urls(LoaderKind::Forge, "1.20.1", "47.2.0");
        assert_eq!(urls.len(), 2);
        assert_eq!(
            urls[0],
            "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"
        );
        assert!(urls[1].starts_with("https://maven.minecraftforge.net/"));
    }

    /// 1.20.1 的 NeoForge 挂在 net/neoforged/forge 下，之后的换成 neoforge。
    #[test]
    fn probe_neoforge_coords() {
        assert_eq!(
            neoforge_coords("1.20.1", "47.1.106"),
            (
                "net/neoforged/forge".to_string(),
                "1.20.1-47.1.106".to_string()
            )
        );
        assert_eq!(
            neoforge_coords("1.21.1", "21.1.80"),
            (
                "net/neoforged/neoforge".to_string(),
                "21.1.80".to_string()
            )
        );
        let urls = installer_urls(LoaderKind::NeoForge, "1.21.1", "21.1.80");
        assert!(urls[0].ends_with(
            "/net/neoforged/neoforge/21.1.80/neoforge-21.1.80-installer.jar"
        ));
    }

    #[test]
    fn probe_profile_urls() {
        let urls = profile_urls(LoaderKind::Fabric, "1.20.1", "0.16.9");
        assert!(urls[0].ends_with("/v2/versions/loader/1.20.1/0.16.9/profile/json"));
        assert!(urls.len() >= 2);
    }

    /// 联网实测：Forge 的 installer 至少在官方或 BMCLAPI 镜像之一能拿到。
    ///
    /// 手动跑：`cargo test --lib probe_installer_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn probe_installer_live_reachable() {
        let client = reqwest::Client::builder()
            .user_agent("NightSpark/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("建 HTTP 客户端失败");

        // Forge / NeoForge 走二进制 installer；Fabric 没有 installer，走 profile json。
        for (kind, mc, loader) in [
            (LoaderKind::Forge, "1.20.1", "47.3.0"),
            (LoaderKind::NeoForge, "1.21.1", "21.1.80"),
            (LoaderKind::Fabric, "1.20.1", "0.16.9"),
        ] {
            let urls = match kind {
                LoaderKind::Fabric | LoaderKind::Quilt => {
                    profile_urls(kind, mc, loader)
                }
                _ => installer_urls(kind, mc, loader),
            };
            let mut ok = false;
            for url in &urls {
                match client.head(url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        println!("{} 可用：{url}", kind.as_str());
                        ok = true;
                        break;
                    }
                    Ok(resp) => println!("{} 返回 {}：{url}", kind.as_str(), resp.status()),
                    Err(err) => println!("{} 请求失败：{url} ({err})", kind.as_str()),
                }
            }
            assert!(ok, "{} 的安装包在官方和镜像上都拿不到", kind.as_str());
        }
    }
}

/// 读整合包导入时写下的加载器要求（`<game_dir>/.nightspark/pack.json`）。
fn read_pack_meta(game_dir: &str) -> Option<(String, String)> {
    let path = Path::new(game_dir).join(".nightspark").join("pack.json");
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let loader = value.get("loader").and_then(Value::as_str)?.trim().to_string();
    if loader.is_empty() {
        return None;
    }
    let mc_version = value
        .get("mc_version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    Some((loader, mc_version))
}
