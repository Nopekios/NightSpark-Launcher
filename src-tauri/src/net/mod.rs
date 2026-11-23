//! 模组站网络层：CurseForge API（key 从 `CURSEFORGE_API_KEY` 读，只挂官方 host，没 key 走
//! mcimirror 反代）、镜像策略、文件直链拼装、Maven 仓库地址。

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;

pub const CF_API: &str = "https://api.curseforge.com";
/// mcimirror 给 CurseForge API 做的反代根（免 key）。
pub const CF_MIRROR: &str = "https://mod.mcimirror.top/curseforge";
pub const CF_CDN: &str = "https://edge.forgecdn.net";
pub const CF_CDN_MIRROR: &str = "https://mod.mcimirror.top";
pub const MCFORGE_MAVEN: &str = "https://maven.minecraftforge.net";
pub const MCFORGE_MAVEN_MIRROR: &str = "https://bmclapi2.bangbang93.com/maven";
pub const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";
pub const FABRIC_META: &str = "https://meta.fabricmc.net";
pub const QUILT_META: &str = "https://meta.quiltmc.org";

/// 一次 API 请求最多带多少个 fileId，官方接口上限 50。
const CF_FILE_CHUNK: usize = 50;
/// 网络超时，和版本下载保持一致。
const HTTP_TIMEOUT_SECS: u64 = 30;

/// 从环境变量读 CurseForge API Key。
///
/// 读不到就返回 `None`，调用方直接走镜像，不去造一个假 key。
pub fn curseforge_key() -> Option<String> {
    for name in ["CURSEFORGE_API_KEY", "NIGHTSPARK_CF_KEY"] {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

pub fn curseforge_api_candidates(path: &str) -> Vec<String> {
    let official = format!("{CF_API}/{}", path.trim_start_matches('/'));
    let mirror = mirror_api(&official);
    if mirror == official {
        vec![official]
    } else {
        vec![official, mirror]
    }
}

/// 把任意 CurseForge API 地址换成镜像地址，规则与 PCL 的 `DlSourceModGet` 一致。
pub fn mirror_api(url: &str) -> String {
    url.replace(CF_API, CF_MIRROR)
}

pub fn mirror_download(url: &str) -> String {
    url.replace("https://cdn.modrinth.com", CF_CDN_MIRROR)
        .replace(CF_CDN, CF_CDN_MIRROR)
}

/// 按 CurseForge 官方规则拼文件名直链。
///
/// 官方规律：fileId 的十进制串前 4 位当一级目录，剩下的当二级目录，
/// 例如 1234567 -> `/files/1234/567/<fileName>`。
pub fn curseforge_cdn_url(file_id: i64, file_name: &str) -> Option<String> {
    if file_id <= 0 || file_name.trim().is_empty() {
        return None;
    }
    let digits = file_id.to_string();
    if digits.len() <= 4 {
        return None;
    }
    let (head, tail) = digits.split_at(4);
    Some(format!(
        "{CF_CDN}/files/{head}/{tail}/{}",
        encode_component(file_name)
    ))
}

pub fn curseforge_download_urls(file_id: i64, file_name: &str) -> Vec<String> {
    let Some(official) = curseforge_cdn_url(file_id, file_name) else {
        return Vec::new();
    };
    let mirror = mirror_download(&official);
    if mirror == official {
        vec![official]
    } else {
        vec![official, mirror]
    }
}

/// 简单 URL 路径段编码：只保留 RFC 3986 的非保留字符，其余转 `%XX`。
pub fn encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Serialize, Clone, Debug)]
pub struct CfFile {
    pub file_id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub display_name: String,
    pub size: u64,
    pub sha1: Option<String>,
    pub folder: String,
    pub urls: Vec<String>,
}

/// 按 `modules` 判断这个文件属于哪一类，规则抄自 PCL 的 `_ResolveCurseForgeTargetFolder`。
fn resolve_folder(file: &Value) -> String {
    let modules: Vec<String> = file
        .get("modules")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(|name| name.to_string())
                .collect()
        })
        .unwrap_or_default();

    if modules.is_empty() {
        return "mods".to_string();
    }
    if modules.iter().any(|name| name == "META-INF" || name == "mcmod.info") {
        return "mods".to_string();
    }
    let file_name = file
        .get("fileName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.ends_with(".jar") {
        return "mods".to_string();
    }
    if modules.iter().any(|name| name == "pack.mcmeta") {
        return "resourcepacks".to_string();
    }
    if modules.iter().any(|name| name == "level.dat") {
        return "saves".to_string();
    }
    "shaderpacks".to_string()
}

/// 从 CF 的 `hashes` 数组里挑 SHA-1（algo = 1）。
fn pick_sha1(file: &Value) -> Option<String> {
    file.get("hashes")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|item| item.get("algo").and_then(Value::as_i64) == Some(1))
                .and_then(|item| item.get("value"))
                .and_then(Value::as_str)
        })
        .map(|value| value.to_ascii_lowercase())
}

fn parse_file(file: &Value) -> Option<CfFile> {
    let file_id = file.get("id").and_then(Value::as_i64)?;
    let file_name = file
        .get("fileName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|name| !name.trim().is_empty())?;

    // 官方 `downloadUrl` 有时是 null（作者关了第三方分发），这时自己拼 CDN 地址。
    let mut urls = Vec::new();
    if let Some(url) = file.get("downloadUrl").and_then(Value::as_str) {
        if !url.trim().is_empty() {
            urls.push(url.to_string());
        }
    }
    for url in curseforge_download_urls(file_id, &file_name) {
        if !urls.iter().any(|item| item.eq_ignore_ascii_case(&url)) {
            urls.push(url);
        }
    }

    Some(CfFile {
        file_id,
        project_id: file.get("modId").and_then(Value::as_i64).unwrap_or(0),
        display_name: file
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(&file_name)
            .to_string(),
        file_name,
        size: file.get("fileLength").and_then(Value::as_u64).unwrap_or(0),
        sha1: pick_sha1(file),
        folder: resolve_folder(file),
        urls,
    })
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent("NightSpark/0.1")
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|err| AppError::Http(format!("创建 HTTP 客户端失败：{err}")))
}

/// 取一段 JSON：能选就挂上 CF 的 key，只往官方 host 挂。
async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    with_key: bool,
) -> Result<Value, AppError> {
    let mut request = client.post(url).json(body);
    if with_key
        && url.starts_with(CF_API)
        && let Some(key) = curseforge_key()
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

/// 批量查 CurseForge 文件信息：官方 API 优先，不够数再补镜像，最后取并集。
///
/// 返回的列表只保证「查到了什么」，顺序与入参无关；调用方按 file_id 自己配对。
pub async fn fetch_curseforge_files(ids: &[i64]) -> Result<Vec<CfFile>, AppError> {
    let mut unique: Vec<i64> = Vec::new();
    for id in ids {
        if *id > 0 && !unique.contains(id) {
            unique.push(*id);
        }
    }
    if unique.is_empty() {
        return Ok(Vec::new());
    }

    let client = http_client()?;
    let mut found: Vec<CfFile> = Vec::new();
    let mut last_error: Option<String> = None;

    for chunk in unique.chunks(CF_FILE_CHUNK) {
        let wanted: Vec<i64> = chunk.to_vec();
        let body = serde_json::json!({ "fileIds": wanted });
        let candidates = curseforge_api_candidates("/v1/mods/files");

        let mut collected: Vec<CfFile> = Vec::new();
        for (index, url) in candidates.iter().enumerate() {
            // 官方接口没 key 时大概率 401，直接跳过省一次往返。
            let is_official = index == 0;
            if is_official && curseforge_key().is_none() {
                continue;
            }
            match post_json(&client, url, &body, is_official).await {
                Ok(value) => {
                    let items = value
                        .get("data")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for item in &items {
                        if let Some(file) = parse_file(item)
                            && !collected.iter().any(|known| known.file_id == file.file_id)
                        {
                            collected.push(file);
                        }
                    }
                    if collected.len() >= wanted.len() {
                        break;
                    }
                }
                Err(err) => last_error = Some(err.to_string()),
            }
        }

        if collected.is_empty() && last_error.is_some() {
            return Err(AppError::Http(format!(
                "查询 CurseForge 文件信息失败：{}",
                last_error.unwrap_or_default()
            )));
        }
        found.extend(collected);
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_curseforge_cdn_url() {
        assert_eq!(
            curseforge_cdn_url(1234567, "Test Mod-1.0.jar").unwrap(),
            "https://edge.forgecdn.net/files/1234/567/Test%20Mod-1.0.jar"
        );
        assert!(curseforge_cdn_url(12, "a.jar").is_none());
        assert!(curseforge_cdn_url(1234567, "  ").is_none());
    }

    #[test]
    fn probe_mirror_rules() {
        assert_eq!(
            mirror_api("https://api.curseforge.com/v1/mods/files"),
            "https://mod.mcimirror.top/curseforge/v1/mods/files"
        );
        assert_eq!(
            mirror_download("https://edge.forgecdn.net/files/1234/567/a.jar"),
            "https://mod.mcimirror.top/files/1234/567/a.jar"
        );
    }

    #[test]
    fn probe_download_urls() {
        let urls = curseforge_download_urls(1234567, "a b.jar");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://edge.forgecdn.net/files/1234/567/"));
        assert!(urls[1].starts_with("https://mod.mcimirror.top/files/1234/567/"));
    }

    #[test]
    fn probe_resolve_folder() {
        let mod_file = serde_json::json!({"modules": [{"name": "META-INF"}, {"name": "mcmod.info"}]});
        assert_eq!(resolve_folder(&mod_file), "mods");
        let pack = serde_json::json!({"modules": [{"name": "pack.mcmeta"}]});
        assert_eq!(resolve_folder(&pack), "resourcepacks");
        let world = serde_json::json!({"modules": [{"name": "level.dat"}]});
        assert_eq!(resolve_folder(&world), "saves");
        let empty = serde_json::json!({"modules": []});
        assert_eq!(resolve_folder(&empty), "mods");
    }

    #[test]
    fn probe_pick_sha1() {
        let file = serde_json::json!({"hashes": [{"algo": 2, "value": "md5"}, {"algo": 1, "value": "ABC"}]});
        assert_eq!(pick_sha1(&file).as_deref(), Some("abc"));
        let none = serde_json::json!({"hashes": []});
        assert!(pick_sha1(&none).is_none());
    }

    /// 联网实测：镜像 API 查文件 -> 拼直链 -> 下载 -> SHA-1 比对。
    ///
    /// 默认忽略，确认 CurseForge 通路是否健康时手动跑：
    /// `cargo test --lib probe_curseforge_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn probe_curseforge_live_roundtrip() {
        let files = fetch_curseforge_files(&[9575423]).await.expect("查文件失败");
        let file = files.first().expect("没查到文件");
        assert!(!file.urls.is_empty(), "CF 文件应当能拼出直链");
        assert!(file.sha1.is_some(), "CF 文件应当带 SHA-1");

        let dir = std::env::temp_dir().join("nightspark-cf-live");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录失败");
        let dest = dir.join(&file.file_name);

        let job = crate::download::FileJob {
            urls: file.urls.clone(),
            dest: dest.clone(),
            size: Some(file.size),
            sha1: file.sha1.clone(),
        };
        let progress = |_: &str, _: u64, _: u64| {};
        let summary = crate::download::download_jobs(vec![job], "测试", &progress)
            .await
            .expect("下载失败");
        assert!(summary.failed.is_empty(), "下载报错：{:?}", summary.failed);
        assert!(dest.exists(), "文件没落盘");

        let actual = crate::hash::sha1_file(&dest).expect("算 SHA-1 失败");
        assert_eq!(
            Some(actual),
            file.sha1.clone(),
            "SHA-1 对不上，直链给的包和清单不一致"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
