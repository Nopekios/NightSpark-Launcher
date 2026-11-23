//! 版本安装模块：拉取版本清单，下载 client jar、libraries、natives 与 assets 到共享 `.minecraft`。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::config;
use crate::error::AppError;

const VERSION_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCE_BASE: &str = "https://resources.download.minecraft.net";
const CONCURRENCY: usize = 16;
/// 单个请求超时（秒）。
const HTTP_TIMEOUT_SECS: u64 = 60;

/// 进度回调：`(阶段, 已完成, 总数)`。
pub type ProgressFn<'a> = &'a (dyn Fn(&str, u64, u64) + Send + Sync);

/// 共享游戏根目录，默认 `%APPDATA%\.minecraft`。
pub fn shared_root() -> Result<PathBuf, AppError> {
    config::minecraft_dir()
}

pub fn versions_dir() -> Result<PathBuf, AppError> {
    Ok(shared_root()?.join("versions"))
}

pub fn version_dir(version_id: &str) -> Result<PathBuf, AppError> {
    Ok(versions_dir()?.join(version_id))
}

pub fn version_json_path(version_id: &str) -> Result<PathBuf, AppError> {
    Ok(version_dir(version_id)?.join(format!("{version_id}.json")))
}

pub fn version_jar_path(version_id: &str) -> Result<PathBuf, AppError> {
    Ok(version_dir(version_id)?.join(format!("{version_id}.jar")))
}

pub fn natives_dir(version_id: &str) -> Result<PathBuf, AppError> {
    Ok(version_dir(version_id)?.join(format!("{version_id}-natives")))
}

pub fn libraries_dir() -> Result<PathBuf, AppError> {
    Ok(shared_root()?.join("libraries"))
}

pub fn assets_dir() -> Result<PathBuf, AppError> {
    Ok(shared_root()?.join("assets"))
}

pub fn indexes_dir() -> Result<PathBuf, AppError> {
    Ok(assets_dir()?.join("indexes"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    /// 最新版本信息，暂未在 UI 中展示，保留以便后续做「一键装最新正式版」。
    #[allow(dead_code)]
    pub latest: LatestVersions,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionMeta {
    pub id: String,
    /// Forge / Fabric 这类版本 json 会继承原版，校验时要顺着往上核。
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(rename = "assetIndex")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub downloads: VersionDownloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersionRef>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaVersionRef {
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgValue>,
    #[serde(default)]
    pub jvm: Vec<ArgValue>,
}

/// 新版参数数组里的元素：纯字符串或带规则的对象。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: StringOrList,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    pub fn push_into(&self, out: &mut Vec<String>, vars: &HashMap<&str, String>) {
        match self {
            StringOrList::One(s) => out.push(substitute(s, vars)),
            StringOrList::Many(list) => {
                for s in list {
                    out.push(substitute(s, vars));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    #[serde(default)]
    pub extract: Option<ExtractRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractRule {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexFile {
    #[serde(default)]
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub version: String,
    pub java_major: u32,
    pub libraries: u32,
    pub assets: u32,
}

/// 当前运行平台的 OS 名称，MC 元数据里 Windows 写作 `windows`。
fn current_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn current_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64") {
        "x86_64"
    } else {
        "x86"
    }
}

/// 判断一条规则是否命中当前环境。`features` 一律视为未启用。
fn rule_matches(rule: &Rule) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != current_os_name() {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != current_arch() {
                return false;
            }
        }
        if let Some(version) = &os.version {
            // 形如 `^10\.` 的正则在标准库之外，这里只做保守放过。
            if version.is_empty() {
                return false;
            }
        }
    }
    if let Some(features) = &rule.features {
        for enabled in features.values() {
            if *enabled {
                return false;
            }
        }
    }
    true
}

/// 按规则数组判断是否允许；空规则视为允许，否则以最后一条命中规则为准。
pub fn rules_allow(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

pub fn library_allowed(library: &Library) -> bool {
    match &library.rules {
        None => true,
        Some(rules) => rules_allow(rules),
    }
}

/// 是否是 natives 包（1.19 之后并入 artifact，名字里带分类器）。
pub fn is_native_library(library: &Library) -> bool {
    if library.natives.is_some() {
        return true;
    }
    let lower = library.name.to_ascii_lowercase();
    lower.contains(":natives-windows")
        || lower.contains(":natives-macos")
        || lower.contains(":natives-linux")
}

/// 判断名字里带分类器的原生包是否属于当前运行平台。
///
/// 1.19 之后版本 json 不再有 `natives` 字段，原生包被拆成
/// `org.lwjgl:lwjgl:3.3.3:natives-windows` 这种独立 library，
/// 平台差异全压在 classifier 上，必须自己挑，否则会把 linux/macos、
/// x86/arm64 的 dll 一并解出来或者一个都不解。
pub fn native_platform_matches(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(idx) = lower.find(":natives-") else {
        return false;
    };
    let classifier = &lower[idx + 1..];
    let mut allowed: Vec<&str> = Vec::new();
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            allowed.push("natives-windows");
        } else if cfg!(target_arch = "aarch64") {
            allowed.push("natives-windows-arm64");
        } else {
            allowed.push("natives-windows-x86");
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            allowed.push("natives-macos-arm64");
            allowed.push("natives-macos");
        } else {
            allowed.push("natives-macos");
        }
    } else if cfg!(target_arch = "aarch64") {
        allowed.push("natives-linux-arm64");
        allowed.push("natives-linux");
    } else {
        allowed.push("natives-linux");
    }
    allowed.iter().any(|a| classifier == *a)
}

/// 把 maven 坐标 `group:artifact:version[:classifier]` 转成相对路径。
pub fn maven_to_path(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 {
        return name.replace(':', "/");
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    format!("{group}/{artifact}/{version}/{artifact}-{version}{classifier}.jar")
}

pub fn library_file_path(library: &Library) -> Result<Option<PathBuf>, AppError> {
    let Some(downloads) = &library.downloads else {
        // 少数库只有 name 没 url（如旧版 natives），按 maven 规则本地路径推测。
        if is_native_library(library) {
            return Ok(None);
        }
        return Ok(Some(libraries_dir()?.join(maven_to_path(&library.name))));
    };
    let rel = match &downloads.artifact {
        Some(artifact) => artifact
            .path
            .clone()
            .unwrap_or_else(|| maven_to_path(&library.name)),
        None => maven_to_path(&library.name),
    };
    Ok(Some(libraries_dir()?.join(rel)))
}

/// 模板变量替换，`${key}` 形式。
pub fn substitute(input: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = input.to_string();
    for (key, value) in vars {
        let token = format!("${{{key}}}");
        if out.contains(&token) {
            out = out.replace(&token, value);
        }
    }
    out
}

pub fn load_local_meta(version_id: &str) -> Result<VersionMeta, AppError> {
    let path = version_json_path(version_id)?;
    let raw = std::fs::read_to_string(&path).map_err(|_| {
        AppError::NotFound(format!(
            "本地没有版本 {version_id} 的元数据，请先下载该版本"
        ))
    })?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::Config(format!("解析版本元数据失败（{}）: {e}", path.display())))
}

/// 读取版本元数据并把 `inheritsFrom` 链上的父版本合并进来。
///
/// Forge / Fabric 的版本 json 只写自己新增的库和参数，原版库、资源索引、
/// Java 版本都在父版本里。启动和补全都必须用合并后的结果，否则 classpath
/// 只有二十来个库，一启动就缺 lwjgl。
pub fn load_merged_meta(version_id: &str) -> Result<VersionMeta, AppError> {
    let mut child = load_local_meta(version_id)?;
    let mut parents: Vec<VersionMeta> = Vec::new();
    let mut cursor = child.inherits_from.clone();
    let mut depth = 0usize;
    while let Some(parent_id) = cursor {
        if depth >= 8 {
            break;
        }
        let parent = load_local_meta(&parent_id)?;
        cursor = parent.inherits_from.clone();
        parents.push(parent);
        depth += 1;
    }
    // 从最顶层父版本开始往下叠，靠近自己的版本胜出。
    for parent in parents.into_iter().rev() {
        merge_parent_meta(&mut child, parent);
    }
    Ok(child)
}

/// 把父版本的信息补进子版本；子版本已有的字段一律不动。
fn merge_parent_meta(child: &mut VersionMeta, parent: VersionMeta) {
    if child.assets.is_none() {
        child.assets = parent.assets;
    }
    if child.asset_index.is_none() {
        child.asset_index = parent.asset_index;
    }
    if child.downloads.client.is_none() {
        child.downloads.client = parent.downloads.client;
    }
    if child.java_version.is_none() {
        child.java_version = parent.java_version;
    }
    if child.kind.is_none() {
        child.kind = parent.kind;
    }
    if child.minecraft_arguments.is_none() {
        child.minecraft_arguments = parent.minecraft_arguments;
    }
    if let Some(parent_arguments) = parent.arguments {
        match child.arguments.as_mut() {
            Some(child_arguments) => {
                // 父版本在前、子版本在后：原版 json 提供 --username / --version /
                // --accessToken / --uuid 这些必需的游戏参数，加载器只追加自己的
                // --launchTarget / --fml.* 与 -p 模块参数。
                // 旧写法是"子版本有 arguments 就整块不动"，NeoForge 自带 arguments 之后
                // 原版那一整组被吞掉，游戏跑到 Main.main 直接
                // MissingRequiredOptionsException: Missing required option(s) [accessToken, version]。
                let mut merged = parent_arguments;
                merged.jvm.append(&mut child_arguments.jvm);
                merged.game.append(&mut child_arguments.game);
                *child_arguments = merged;
            }
            None => child.arguments = Some(parent_arguments),
        }
    }
    // 库按 maven 坐标去重：父在前，子同名的覆盖父。
    let mut merged: Vec<Library> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for library in parent.libraries {
        if seen.insert(library.name.clone()) {
            merged.push(library);
        }
    }
    for library in child.libraries.drain(..) {
        if seen.insert(library.name.clone()) {
            merged.push(library);
        }
    }
    child.libraries = merged;
}

/// 判断版本是否已安装到本地（jar 与 json 都在）。
pub fn is_installed(version_id: &str) -> bool {
    version_json_path(version_id)
        .map(|p| p.is_file())
        .unwrap_or(false)
        && version_jar_path(version_id)
            .map(|p| p.is_file())
            .unwrap_or(false)
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent("NightSpark/0.1")
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::Http(format!("创建 HTTP 客户端失败: {e}")))
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("请求失败 {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "请求失败 {url}: HTTP {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| AppError::Http(format!("读取响应失败 {url}: {e}")))
}

pub async fn fetch_manifest() -> Result<VersionManifest, AppError> {
    let client = http_client()?;
    let raw = fetch_text(&client, VERSION_MANIFEST_URL).await?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::Config(format!("解析版本清单失败: {e}")))
}

#[derive(Default)]
struct DownloadTask {
    url: String,
    dest: PathBuf,
    size: Option<u64>,
    /// 期望的 SHA-1；给了就在落盘前校验，对不上直接判失败并把 .part 丢掉。
    sha1: Option<String>,
}

async fn download_one(client: &reqwest::Client, task: &DownloadTask) -> Result<(), AppError> {
    // 已存在且大小对得上就跳过，支持断点重跑；大小对得上但哈希不对的情况交给调用方专项校验。
    if let Ok(meta) = std::fs::metadata(&task.dest) {
        let ok = match task.size {
            Some(size) => meta.len() == size,
            None => meta.len() > 0,
        };
        if ok {
            return Ok(());
        }
    }
    if let Some(parent) = task.dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let resp = client
        .get(&task.url)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("下载失败 {}: {e}", task.url)))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "下载失败 {}: HTTP {}",
            task.url,
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Http(format!("读取下载内容失败 {}: {e}", task.url)))?;
    // 有期望哈希就当场核对，宁可抛错也不把坏文件当成装好了。
    if let Some(expected) = task.sha1.as_deref() {
        if crate::hash::supported_sha1(expected).is_none() {
            return Err(AppError::Config(format!(
                "{} 的期望哈希 {expected} 不是 SHA-1，无法校验",
                task.dest.display()
            )));
        }
        let actual = crate::hash::sha1_bytes(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(AppError::Http(format!(
                "{} 校验失败：期望 SHA-1 {expected}，实际 {actual}（源 {}）",
                task.dest.display(),
                task.url
            )));
        }
    }
    let tmp = with_part_suffix(&task.dest);
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &task.dest)?;
    Ok(())
}

fn with_part_suffix(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    name.push_str(".part");
    path.with_file_name(name)
}

async fn download_batch(
    tasks: Vec<DownloadTask>,
    stage: &str,
    progress: ProgressFn<'_>,
) -> Result<(), AppError> {
    let total = tasks.len() as u64;
    if total == 0 {
        progress(stage, 0, 0);
        return Ok(());
    }

    let client = Arc::new(http_client()?);
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let done = Arc::new(AtomicU64::new(0));
    let step = (total / 40).max(1);

    let mut handles = Vec::with_capacity(tasks.len());
    for task in tasks {
        let client = client.clone();
        let sem = sem.clone();
        let done = done.clone();
        handles.push(tokio::spawn(async move {
            let permit = sem.acquire().await;
            if permit.is_err() {
                return (Err(AppError::Internal("并发控制失败".to_string())), 0);
            }
            let result = download_one(&client, &task).await;
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            (result, finished)
        }));
    }

    let mut errors: Vec<String> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((result, finished)) => {
                if finished % step == 0 || finished == total {
                    progress(stage, finished, total);
                }
                if let Err(e) = result {
                    errors.push(e.to_string());
                }
            }
            Err(e) => errors.push(format!("下载任务异常: {e}")),
        }
    }
    progress(stage, total, total);

    if errors.is_empty() {
        Ok(())
    } else {
        let first = errors.first().cloned().unwrap_or_default();
        Err(AppError::Http(format!(
            "{} 个文件下载失败，首个错误：{}",
            errors.len(),
            first
        )))
    }
}

/// 一个待下载文件：候选源可以给多个，按顺序重试。
pub struct FileJob {
    pub urls: Vec<String>,
    pub dest: PathBuf,
    pub size: Option<u64>,
    pub sha1: Option<String>,
}

#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct DownloadSummary {
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    /// 失败项在入参里的下标，与 `failed` 一一对应，方便调用方回写清单。
    pub failed_index: Vec<usize>,
}

/// 与内部 `download_batch` 的区别：每个文件可以带一串候选源（官方 + 镜像），
/// 前一个源失败会自动换下一个；本地已有文件会先做大小 + SHA-1 校验，
/// 校验不过就当没下过，重新拉到对为止。
pub async fn download_jobs(
    jobs: Vec<FileJob>,
    stage: &str,
    progress: ProgressFn<'_>,
) -> Result<DownloadSummary, AppError> {
    let total = jobs.len() as u64;
    // 没有作业就别上报 0/0 进度：这种事件既算不出百分比、也判定不了完成，
    // 前端任务条会一直挂着，看起来像死锁。
    if total == 0 {
        return Ok(DownloadSummary::default());
    }

    let client = Arc::new(http_client()?);
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let done = Arc::new(AtomicU64::new(0));
    let step = (total / 40).max(1);

    let mut handles = Vec::with_capacity(jobs.len());
    for job in jobs {
        let client = client.clone();
        let sem = sem.clone();
        let done = done.clone();
        handles.push(tokio::spawn(async move {
            if sem.acquire().await.is_err() {
                return (Err("并发控制失败".to_string()), 0u64, false);
            }

            // 本地已有：大小对、哈希对，才算「已经好了」。
            let mut satisfied = false;
            if job.dest.is_file() {
                let size_ok = match job.size {
                    Some(size) => std::fs::metadata(&job.dest)
                        .map(|meta| meta.len() == size)
                        .unwrap_or(false),
                    None => true,
                };
                let hash_ok = match job.sha1.as_deref() {
                    Some(expected) => crate::hash::verify_file(&job.dest, expected).unwrap_or(false),
                    None => true,
                };
                satisfied = size_ok && hash_ok;
            }

            let mut last_error: Option<String> = None;
            let mut downloaded = false;
            if !satisfied {
                if job.urls.is_empty() {
                    last_error = Some(format!("{} 没有任何可用下载地址", job.dest.display()));
                } else {
                    for url in &job.urls {
                        let task = DownloadTask {
                            url: url.clone(),
                            dest: job.dest.clone(),
                            size: job.size,
                            sha1: job.sha1.clone(),
                        };
                        match download_one(&client, &task).await {
                            Ok(()) => {
                                last_error = None;
                                downloaded = true;
                                break;
                            }
                            Err(err) => {
                                // 校验失败或半截文件留着只会误导后续判断，直接删掉再换源。
                                let _ = std::fs::remove_file(&job.dest);
                                let _ = std::fs::remove_file(with_part_suffix(&job.dest));
                                last_error = Some(err.to_string());
                            }
                        }
                    }
                }
            }

            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            match last_error {
                Some(error) => (Err(error), finished, false),
                None => (Ok(()), finished, if downloaded { false } else { true }),
            }
        }));
    }

    let mut summary = DownloadSummary::default();
    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok((result, finished, skipped)) => {
                if finished % step == 0 || finished == total {
                    progress(stage, finished, total);
                }
                match result {
                    Ok(()) => {
                        if skipped {
                            summary.skipped += 1;
                        } else {
                            summary.downloaded += 1;
                        }
                    }
                    Err(error) => {
                        summary.failed.push(error);
                        summary.failed_index.push(index);
                    }
                }
            }
            Err(error) => {
                summary.failed.push(format!("下载任务异常：{error}"));
                summary.failed_index.push(index);
            }
        }
    }
    progress(stage, total, total);
    Ok(summary)
}

fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<(), AppError> {
    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Config(format!("打开 {} 失败: {e}", jar.display())))?;

    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Some(name) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let name_str = name.to_string_lossy().replace('\\', "/");
        if name_str.starts_with("META-INF/") {
            continue;
        }
        if exclude.iter().any(|ex| name_str.starts_with(ex)) {
            continue;
        }
        let target = dest.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer)?;
        std::fs::write(&target, &buffer)?;
    }
    Ok(())
}

pub async fn install_version(
    version_id: &str,
    with_assets: bool,
    progress: ProgressFn<'_>,
) -> Result<InstallReport, AppError> {
    progress("manifest", 0, 1);
    let manifest = fetch_manifest().await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| AppError::NotFound(format!("版本清单里没有 {version_id}")))?
        .clone();
    progress("manifest", 1, 1);

    let client = http_client()?;
    let raw = fetch_text(&client, &entry.url).await?;
    let meta: VersionMeta = serde_json::from_str(&raw)
        .map_err(|e| AppError::Config(format!("解析版本元数据失败: {e}")))?;

    let vdir = version_dir(version_id)?;
    std::fs::create_dir_all(&vdir)?;
    std::fs::write(version_json_path(version_id)?, &raw)?;

    let client_artifact = meta
        .downloads
        .client
        .as_ref()
        .ok_or_else(|| AppError::Config(format!("版本 {version_id} 没有 client 下载项")))?;
    download_batch(
        vec![DownloadTask {
            url: client_artifact.url.clone(),
            dest: version_jar_path(version_id)?,
            size: client_artifact.size,
            ..DownloadTask::default()
        }],
        "client",
        progress,
    )
    .await?;

    let libs_root = libraries_dir()?;
    let natives_root = natives_dir(version_id)?;
    let mut lib_tasks = Vec::new();
    let mut native_jars: Vec<PathBuf> = Vec::new();
    let mut excludes: Vec<String> = Vec::new();
    let mut lib_count = 0u32;

    for library in &meta.libraries {
        if !library_allowed(library) {
            continue;
        }
        if let Some(extract) = &library.extract {
            for item in &extract.exclude {
                if !excludes.contains(item) {
                    excludes.push(item.clone());
                }
            }
        }

        let push_artifact = |artifact: &Artifact,
                                 tasks: &mut Vec<DownloadTask>,
                                 natives: &mut Vec<PathBuf>| {
            let rel = artifact
                .path
                .clone()
                .unwrap_or_else(|| maven_to_path(&library.name));
            let dest = libs_root.join(&rel);
            tasks.push(DownloadTask {
                url: artifact.url.clone(),
                dest: dest.clone(),
                size: artifact.size,
                ..DownloadTask::default()
            });
            if is_native_library(library) {
                natives.push(dest);
            }
        };

        let Some(downloads) = &library.downloads else {
            continue;
        };

        if let Some(artifact) = &downloads.artifact {
            push_artifact(artifact, &mut lib_tasks, &mut native_jars);
            lib_count += 1;
        }

        // 旧版（1.12 及以前）natives 放在 classifiers 里。
        if let (Some(classifiers), Some(natives)) = (&downloads.classifiers, &library.natives)
            && let Some(key) = natives.get("windows")
        {
            let key = key.replace("${arch}", "64");
            if let Some(artifact) = classifiers.get(&key) {
                let rel = artifact
                    .path
                    .clone()
                    .unwrap_or_else(|| maven_to_path(&library.name));
                lib_tasks.push(DownloadTask {
                    url: artifact.url.clone(),
                    dest: libs_root.join(&rel),
                    size: artifact.size,
                    ..DownloadTask::default()
                });
                native_jars.push(libs_root.join(rel));
            }
        }
    }

    download_batch(lib_tasks, "libraries", progress).await?;

    if !native_jars.is_empty() {
        progress("natives", 0, native_jars.len() as u64);
        for (index, jar) in native_jars.iter().enumerate() {
            extract_natives(jar, &natives_root, &excludes)?;
            progress("natives", index as u64 + 1, native_jars.len() as u64);
        }
    }

    let mut asset_count = 0u32;
    if let Some(index_ref) = &meta.asset_index {
        let index_path = indexes_dir()?.join(format!("{}.json", index_ref.id));
        download_batch(
            vec![DownloadTask {
                url: index_ref.url.clone(),
                dest: index_path.clone(),
                size: index_ref.size,
                sha1: None,
            }],
            "assets-index",
            progress,
        )
        .await?;

        if with_assets {
            let raw_index = std::fs::read_to_string(&index_path)?;
            let index: AssetIndexFile = serde_json::from_str(&raw_index)
                .map_err(|e| AppError::Config(format!("解析资源索引失败: {e}")))?;
            let objects_root = assets_dir()?.join("objects");
            let mut asset_tasks = Vec::with_capacity(index.objects.len());
            for object in index.objects.values() {
                if object.hash.len() < 2 {
                    continue;
                }
                let prefix = &object.hash[..2];
                asset_tasks.push(DownloadTask {
                    url: format!("{RESOURCE_BASE}/{prefix}/{}", object.hash),
                    dest: objects_root.join(prefix).join(&object.hash),
                    size: Some(object.size),
                    // 资源对象的文件名本身就是 SHA-1，顺手把校验做掉。
                    sha1: Some(object.hash.clone()),
                });
            }
            asset_count = asset_tasks.len() as u32;
            download_batch(asset_tasks, "assets", progress).await?;
        }
    }

    Ok(InstallReport {
        version: meta.id.clone(),
        java_major: meta
            .java_version
            .as_ref()
            .map(|v| v.major_version)
            .unwrap_or(17),
        libraries: lib_count,
        assets: asset_count,
    })
}

/// 把版本需要的原生库（lwjgl 的 dll）解压到 `versions/<版本>/natives`。
///
/// 手动导入的整合包、以及我们自己解包装的 Forge，都没走完整安装流程，
/// natives 目录是空的，一启动就报缺 lwjgl64.dll。启动前补一遍，重复执行无害。
pub fn ensure_natives(version_id: &str) -> Result<u32, AppError> {
    let meta = load_merged_meta(version_id)?;
    let natives_root = natives_dir(version_id)?;
    std::fs::create_dir_all(&natives_root)?;
    let libs_root = libraries_dir()?;
    let mut extracted = 0u32;
    for library in &meta.libraries {
        if !library_allowed(library) {
            continue;
        }
        // 1.19 之后：版本 json 里没有 natives 字段，原生包是一条独立的 library，
        // 平台差异写在 classifier（name 的第 4 段）里，必须按当前平台挑。
        if library.natives.is_none() {
            if !is_native_library(library) || !native_platform_matches(&library.name) {
                continue;
            }
            let rel = match library_file_path(library)? {
                Some(path) => path,
                None => PathBuf::from(maven_to_path(&library.name)),
            };
            let jar = if rel.is_absolute() {
                rel
            } else {
                libs_root.join(rel)
            };
            if jar.is_file() {
                extract_natives(&jar, &natives_root, &[])?;
                extracted += 1;
            }
            continue;
        }
        let mut jars: Vec<PathBuf> = Vec::new();
        if let Some(downloads) = &library.downloads {
            // 1.12 及以前：原生包放在 classifiers 里，按 natives 的键取。
            if let (Some(classifiers), Some(natives)) = (&downloads.classifiers, &library.natives) {
                if let Some(key) = natives.get("windows") {
                    let key = key.replace("${arch}", "64");
                    if let Some(artifact) = classifiers.get(&key) {
                        let rel = artifact.path.clone().unwrap_or_else(|| {
                            maven_to_path(&format!("{}:{}", library.name, key))
                        });
                        jars.push(libs_root.join(rel));
                    }
                }
            }
            // 1.19 之后：原生包就是 artifact 本身。
            if let Some(artifact) = &downloads.artifact {
                let rel = artifact
                    .path
                    .clone()
                    .unwrap_or_else(|| maven_to_path(&library.name));
                jars.push(libs_root.join(rel));
            }
        }
        let exclude = library
            .extract
            .as_ref()
            .map(|rule| rule.exclude.clone())
            .unwrap_or_default();
        for jar in jars {
            if jar.is_file() {
                extract_natives(&jar, &natives_root, &exclude)?;
                extracted += 1;
            }
        }
    }
    Ok(extracted)
}

pub fn collect_classpath(meta: &VersionMeta, version_id: &str) -> Result<Vec<PathBuf>, AppError> {
    let mut paths = Vec::new();
    for library in &meta.libraries {
        if !library_allowed(library) || is_native_library(library) {
            continue;
        }
        if let Some(path) = library_file_path(library)? {
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    let client_jar = version_jar_path(version_id)?;
    if client_jar.is_file() {
        paths.push(client_jar);
    }
    Ok(paths)
}

#[cfg(test)]
mod merge_args_tests {
    use super::*;

    fn parse(json: &str) -> VersionMeta {
        serde_json::from_str(json).expect("parse version meta")
    }

    /// 加载器的 json 自带 arguments 时，原版的 game 参数不能被吞掉，
    /// 否则 --accessToken / --version 缺失，Main.main 直接抛
    /// MissingRequiredOptionsException。父在前、子在后。
    #[test]
    fn keeps_parent_game_arguments() {
        let parent = parse(
            r#"{
                "id": "1.21.1",
                "mainClass": "net.minecraft.client.main.Main",
                "arguments": {
                    "game": ["--username", "${auth_player_name}", "--version", "${version_name}",
                             "--accessToken", "${auth_access_token}"],
                    "jvm": ["-Dparent=1"]
                }
            }"#,
        );
        let mut child = parse(
            r#"{
                "id": "neoforge-21.1.266",
                "inheritsFrom": "1.21.1",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
                "arguments": {
                    "game": ["--launchTarget", "forgeclient"],
                    "jvm": ["-Dchild=1"]
                }
            }"#,
        );
        merge_parent_meta(&mut child, parent);
        let arguments = child.arguments.expect("merged arguments");
        let plain: Vec<String> = arguments
            .game
            .iter()
            .filter_map(|arg| match arg {
                ArgValue::Plain(s) => Some(s.clone()),
                ArgValue::Conditional { .. } => None,
            })
            .collect();
        assert!(plain.iter().any(|s| s == "--accessToken"));
        assert!(plain.iter().any(|s| s == "--version"));
        assert!(plain.iter().any(|s| s == "--launchTarget"));
        let token_pos = plain.iter().position(|s| s == "--accessToken").unwrap();
        let target_pos = plain.iter().position(|s| s == "--launchTarget").unwrap();
        assert!(token_pos < target_pos, "父参数应排在子参数之前");
        assert_eq!(arguments.jvm.len(), 2, "jvm 参数应父子都保留");
    }
}

#[cfg(test)]
mod native_platform_tests {
    use super::*;

    /// 1.19+ 的 natives 只靠 name 里的 classifier 区分平台，
    /// 必须一个平台只挑一个包，否则会把别的系统的 dll 解到 natives 目录里。
    #[test]
    fn picks_only_current_platform() {
        let native_name = "org.lwjgl:lwjgl:3.3.3:natives-windows";
        let plain = "org.lwjgl:lwjgl:3.3.3";
        let arm64 = "org.lwjgl:lwjgl:3.3.3:natives-windows-arm64";
        let x86 = "org.lwjgl:lwjgl:3.3.3:natives-windows-x86";
        let linux = "org.lwjgl:lwjgl:3.3.3:natives-linux";
        let macos = "org.lwjgl:lwjgl:3.3.3:natives-macos-arm64";

        if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
            assert!(native_platform_matches(native_name));
        }
        assert!(!native_platform_matches(plain));
        assert!(!native_platform_matches(arm64));
        assert!(!native_platform_matches(x86));
        assert!(!native_platform_matches(linux));
        assert!(!native_platform_matches(macos));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// 真实网络验证：cargo test --lib live_install -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_install_1_20_1() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build runtime");
        let outcome = rt.block_on(async {
            install_version("1.20.1", false, &|stage: &str, done: u64, total: u64| {
                if total > 0 && (done == total || done % 40 == 0) {
                    println!("[progress] {stage} {done}/{total}");
                }
            })
            .await
        });
        let report = outcome.unwrap_or_else(|err| panic!("install failed: {err}"));
        println!(
            "installed version={} java_major={} libraries={} assets={}",
            report.version, report.java_major, report.libraries, report.assets
        );
        println!("version dir = {}", version_dir("1.20.1").unwrap().display());
        println!("jar exists = {}", version_jar_path("1.20.1").unwrap().exists());
    }
}
