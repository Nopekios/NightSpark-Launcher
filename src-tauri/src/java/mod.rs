//! Java 运行时模块：按需下载 Mojang 官方 JRE，与游戏本体同源同 CDN。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::download::{self, FileJob, ProgressFn};
use crate::error::AppError;

const RUNTIME_INDEX: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JavaRuntime {
    pub major: u32,
    pub path: String,
    /// 官方清单里记录的版本名，如 `8u51-cacert462b08`。
    pub version: String,
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent(concat!("NightSpark/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Internal(format!("创建 HTTP 客户端失败: {e}")))
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("请求 {url} 失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "请求 {url} 返回 {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| AppError::Http(format!("读取 {url} 响应失败: {e}")))
}

/// Java 运行时的落地根目录：`config_dir/NightSpark/runtimes`。
pub fn runtimes_root() -> Result<PathBuf, AppError> {
    let path = crate::config::launcher_config_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| AppError::Internal("配置目录解析失败".to_string()))?
        .join("runtimes");
    Ok(dir)
}

pub fn runtime_dir(major: u32) -> Result<PathBuf, AppError> {
    Ok(runtimes_root()?.join(format!("java-{major}")))
}

/// 平台键，Mojang 清单里用的是 `windows-x64` 这种写法。
fn platform_key() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "windows-arm64"
        } else if cfg!(target_arch = "x86") {
            "windows-x86"
        } else {
            "windows-x64"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "mac-os-arm64"
        } else {
            "mac-os"
        }
    } else {
        "linux"
    }
}

fn java_exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// Java 主版本 → Mojang 组件名。
///
/// 官方清单里没有 9~15 的通用组件，遇到就退化到相邻版本：
/// 8 及以下给 `jre-legacy`，9~16 给 16，17~20 给 17，21~24 给 21，25+ 给 25。
pub fn component_for(major: u32) -> &'static str {
    match major {
        0..=8 => "jre-legacy",
        9..=16 => "java-runtime-alpha",
        17..=20 => "java-runtime-beta",
        21..=24 => "java-runtime-delta",
        _ => "java-runtime-epsilon",
    }
}

/// 组件实际提供的主版本，用于校验（避免"要 8 给 16"这种错配）。
fn component_major(component: &str) -> u32 {
    match component {
        "jre-legacy" => 8,
        "java-runtime-alpha" => 16,
        "java-runtime-beta" | "java-runtime-gamma" | "java-runtime-gamma-snapshot" => 17,
        "java-runtime-delta" => 21,
        _ => 25,
    }
}

pub fn exe_in(dir: &PathBuf) -> PathBuf {
    dir.join("bin").join(java_exe_name())
}

/// 扫描本地已装好的 Java 运行时（只认本模块下过的）。
pub fn installed() -> Vec<JavaRuntime> {
    let Ok(root) = runtimes_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut out: Vec<JavaRuntime> = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(major) = name
            .strip_prefix("java-")
            .and_then(|s| s.trim().parse::<u32>().ok())
        else {
            continue;
        };
        let exe = exe_in(&dir);
        if !exe.is_file() {
            continue;
        }
        out.push(JavaRuntime {
            major,
            path: exe.to_string_lossy().to_string(),
            version: read_release_name(&dir).unwrap_or_else(|| format!("java {major}")),
        });
    }
    out.sort_by_key(|j| j.major);
    out
}

/// 从运行时目录里的 `release` 文件读版本名，读不到就算了。
fn read_release_name(dir: &PathBuf) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("release")).ok()?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("JAVA_VERSION=") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// 下载并安装指定主版本的 Java 运行时。
///
/// 全程走 `download_jobs`，每个文件带 SHA-1 校验，进度通过 `progress`
/// 回调抛给前端（阶段名：`准备 Java 运行时` → `下载 Java 运行时` → 完成）。
pub async fn install(major: u32, progress: ProgressFn<'_>) -> Result<JavaRuntime, AppError> {
    let component = component_for(major);
    let actual_major = component_major(component);
    if actual_major > major + 3 {
        return Err(AppError::Config(format!(
            "官方清单里没有正好 Java {major} 的运行时，最接近的是 Java {actual_major}，请改选这个版本"
        )));
    }

    let root = runtime_dir(major)?;
    let exe = exe_in(&root);
    if exe.is_file() {
        progress("Java 运行时已就绪", 1, 1);
        return Ok(JavaRuntime {
            major,
            path: exe.to_string_lossy().to_string(),
            version: read_release_name(&root).unwrap_or_else(|| format!("java {major}")),
        });
    }

    progress("准备 Java 运行时", 0, 1);
    let client = http_client()?;
    let index_raw = fetch_text(&client, RUNTIME_INDEX).await?;
    let index: Value = serde_json::from_str(&index_raw)
        .map_err(|e| AppError::Http(format!("解析 Java 运行时清单失败: {e}")))?;

    let platform = platform_key();
    let entry = index
        .get(platform)
        .and_then(|v| v.get(component))
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "官方 Java 清单里没有 {component} 的 {platform} 版本，这个平台可能不支持自动安装"
            ))
        })?;

    let manifest_url = entry
        .get("manifest")
        .and_then(|m| m.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| AppError::Http("Java 清单里没给 manifest 地址".to_string()))?;
    let version_name = entry
        .get("version")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let manifest_raw = fetch_text(&client, manifest_url).await?;
    let manifest: Value = serde_json::from_str(&manifest_raw)
        .map_err(|e| AppError::Http(format!("解析 Java 文件清单失败: {e}")))?;
    let files = manifest
        .get("files")
        .and_then(|f| f.as_object())
        .ok_or_else(|| AppError::Http("Java 文件清单是空的".to_string()))?;

    let mut jobs: Vec<FileJob> = Vec::new();
    for (rel, info) in files {
        if info.get("type").and_then(|t| t.as_str()) != Some("file") {
            continue;
        }
        let Some(raw) = info.get("downloads").and_then(|d| d.get("raw")) else {
            continue;
        };
        let Some(url) = raw.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let size = raw.get("size").and_then(|s| s.as_u64());
        let sha1 = raw
            .get("sha1")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let rel_path = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        jobs.push(FileJob {
            urls: vec![url.to_string()],
            dest: root.join(rel_path),
            size,
            sha1,
        });
    }

    if jobs.is_empty() {
        return Err(AppError::Http(
            "官方 Java 清单里没解析出可下载的文件".to_string(),
        ));
    }

    let total = jobs.len() as u64;
    progress("下载 Java 运行时", 0, total);
    let summary = download::download_jobs(jobs, "下载 Java 运行时", progress).await?;
    if !summary.failed.is_empty() {
        return Err(AppError::Http(format!(
            "Java 运行时下载没完成，{} 个文件失败：{}",
            summary.failed.len(),
            summary
                .failed
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("；")
        )));
    }

    if !exe.is_file() {
        return Err(AppError::Http(
            "Java 运行时解包后没找到 java 可执行文件，可能下载不完整".to_string(),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }

    progress("下载 Java 运行时", total, total);
    Ok(JavaRuntime {
        major,
        path: exe.to_string_lossy().to_string(),
        version: if version_name.is_empty() {
            format!("java {major}")
        } else {
            version_name
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_component_mapping() {
        assert_eq!(component_for(8), "jre-legacy");
        assert_eq!(component_for(16), "java-runtime-alpha");
        assert_eq!(component_for(17), "java-runtime-beta");
        assert_eq!(component_for(21), "java-runtime-delta");
        assert_eq!(component_major("jre-legacy"), 8);
        assert_eq!(component_major("java-runtime-delta"), 21);
    }

    #[test]
    fn probe_platform_key() {
        assert!(platform_key().starts_with("windows") || platform_key() == "linux");
    }

    /// 联网真装 Java 8，验证清单解析 + 逐文件校验下载。默认忽略。
    #[tokio::test]
    #[ignore]
    async fn live_install_java8() {
        let sink = |stage: &str, done: u64, total: u64| {
            println!("[progress] {stage} {done}/{total}");
        };
        // 清掉旧目录，保证走完整下载流程。
        let _ = std::fs::remove_dir_all(runtime_dir(8).unwrap());
        let rt = install(8, &sink).await.expect("安装 Java 8 失败");
        println!("installed: {:?}", rt);
        assert!(std::path::Path::new(&rt.path).is_file());
    }
}
