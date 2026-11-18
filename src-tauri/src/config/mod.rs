//! 启动器配置模块：路径解析、配置读写、本机 Java 探测。

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// 启动器配置。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LauncherConfig {
    /// Java 可执行文件路径，None 表示由启动器自动探测
    pub java_path: Option<String>,
    /// 分配给游戏的最大内存（MB）
    pub memory_mb: u32,
    /// 游戏目录覆盖值，None 表示使用默认 .minecraft
    pub game_dir: Option<String>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            java_path: None,
            memory_mb: 4096,
            game_dir: None,
        }
    }
}

/// 探测到的 Java 运行时。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JavaInfo {
    pub path: String,
    pub version: String,
}

/// 当前平台的 java 可执行文件名。
fn java_exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 解析 Minecraft 游戏目录：MINECRAFT_DIR 优先，否则 `data_dir/.minecraft`。
///
/// 只返回路径，不创建目录。
pub fn minecraft_dir() -> Result<PathBuf, AppError> {
    if let Ok(dir) = env::var("MINECRAFT_DIR") {
        let dir = dir.trim().trim_matches('"');
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }

    let data_dir = dirs::data_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 data 目录".to_string()))?;
    Ok(data_dir.join(".minecraft"))
}

/// 启动器自身配置文件路径：`config_dir/Love-Launcher/config.json`。
///
/// 刻意不放进 `.minecraft`，避免与游戏数据混在一起。
pub fn launcher_config_path() -> Result<PathBuf, AppError> {
    if let Ok(home) = env::var("LOVE_LAUNCHER_HOME") {
        let home = home.trim().trim_matches('"');
        if !home.is_empty() {
            return Ok(PathBuf::from(home).join("config.json"));
        }
    }

    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 config 目录".to_string()))?;
    Ok(config_dir.join("Love-Launcher").join("config.json"))
}

/// 读取启动器配置；文件不存在时返回默认配置。
pub fn load_config() -> Result<LauncherConfig, AppError> {
    let path = launcher_config_path()?;
    if !path.exists() {
        return Ok(LauncherConfig::default());
    }

    let raw = std::fs::read_to_string(&path)?;
    let cfg = serde_json::from_str(&raw).map_err(|err| {
        AppError::Config(format!("解析配置文件失败（{}）: {}", path.display(), err))
    })?;
    Ok(cfg)
}

/// 保存启动器配置，自动创建父目录。
pub fn save_config(cfg: &LauncherConfig) -> Result<(), AppError> {
    let path = launcher_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(cfg)
        .map_err(|err| AppError::Config(format!("序列化配置失败: {}", err)))?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// 收集候选 java 可执行文件路径（JAVA_HOME 优先，然后扫 PATH）。
fn java_candidates() -> Vec<PathBuf> {
    let exe = java_exe_name();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(home) = env::var("JAVA_HOME") {
        let home = home.trim().trim_matches('"');
        if !home.is_empty() {
            candidates.push(Path::new(home).join("bin").join(exe));
        }
    }

    let sep = if cfg!(windows) { ';' } else { ':' };
    if let Ok(path_var) = env::var("PATH") {
        for entry in path_var.split(sep) {
            let entry = entry.trim().trim_matches('"');
            if entry.is_empty() {
                continue;
            }
            candidates.push(Path::new(entry).join(exe));
        }
    }

    candidates
}

/// 兜底扫描用的根目录列表。
///
/// 用途：从 IDE、旧 shell 或受限环境启动时，进程环境里可能没有
/// `JAVA_HOME`、PATH 里也没有 java，这时靠扫常见安装位置兜底。
fn fallback_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if cfg!(windows) {
        const FIXED: [&str; 7] = [
            r"C:\Program Files\Java",
            r"C:\Program Files (x86)\Java",
            r"C:\Program Files\Eclipse Adoptium",
            r"C:\Program Files (x86)\Eclipse Adoptium",
            r"C:\Program Files\Amazon Corretto",
            r"C:\Program Files\Zulu",
            r"C:\Dev\JDK",
        ];
        for dir in FIXED {
            roots.push(PathBuf::from(dir));
        }

        // C:\Program Files\Microsoft\jdk-* 这种带版本号的供应商目录
        let microsoft_root = Path::new(r"C:\Program Files\Microsoft");
        for entry in read_dir_ok(microsoft_root) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let is_jdk = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase().starts_with("jdk-"))
                .unwrap_or(false);
            if is_jdk {
                roots.push(path);
            }
        }

        // 用户级目录：不依赖环境变量，优先用 dirs 提供的路径
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".jdks"));
        }
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Programs").join("Eclipse Adoptium"));
        }
    } else {
        roots.push(PathBuf::from("/usr/lib/jvm"));
        roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
    }

    // 启动器自己下过的运行时也要能被探测到。
    if let Ok(dir) = crate::java::runtimes_root() {
        roots.push(dir);
    }
    roots
}

/// `read_dir` 的容错封装：目录不存在或不可读时返回空列表，不报错。
fn read_dir_ok(dir: &Path) -> Vec<std::fs::DirEntry> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

/// 在单个根目录下寻找 java 可执行文件，最多下探两层。
///
/// - 根目录自己就是 JDK 根（`root/bin/java`）→ 直接收下，不再下探
/// - `root/<jdk>/bin/java` → 收下
/// - `root/<vendor>/<jdk>/bin/java` → 收下
fn push_java_in_dir(root: &Path, candidates: &mut Vec<PathBuf>) {
    if !root.is_dir() {
        return;
    }

    let exe = java_exe_name();
    let direct = root.join("bin").join(exe);
    if direct.is_file() {
        candidates.push(direct);
        return;
    }

    for entry in read_dir_ok(root) {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }

        let child_exe = child.join("bin").join(exe);
        if child_exe.is_file() {
            candidates.push(child_exe);
            continue;
        }

        for entry2 in read_dir_ok(&child) {
            let grandchild = entry2.path();
            if !grandchild.is_dir() {
                continue;
            }
            let grandchild_exe = grandchild.join("bin").join(exe);
            if grandchild_exe.is_file() {
                candidates.push(grandchild_exe);
            }
        }
    }
}

/// 从 `java -version` 输出中解析版本号（形如 `version "17.0.9"`）。
fn parse_java_version(output: &str) -> Option<String> {
    const MARKER: &str = "version \"";
    let start = output.find(MARKER)? + MARKER.len();
    let rest = &output[start..];
    let end = rest.find('"')?;
    let version = rest[..end].trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// 把 `1.8.0_51` / `17.0.9` / `21` 这类版本字符串归一到主版本号。
pub fn major_from_version(version: &str) -> Option<u32> {
    let mut parts = version.split(['.', '_', '-']);
    let first = parts.next()?.trim().parse::<u32>().ok()?;
    if first == 1 {
        // 1.8.0_51 这种旧式写法，真正的主版本在第二段。
        parts.next()?.trim().split('.').next()?.parse::<u32>().ok()
    } else {
        Some(first)
    }
}

/// 探测某个 java 可执行文件的主版本号（8 / 17 / 21 …）。
///
/// 解析不出来返回 None，调用方按「不拦」处理。
pub fn probe_major(path: &str) -> Option<u32> {
    let output = Command::new(path).arg("-version").output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stderr).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    major_from_version(&parse_java_version(&text)?)
}

/// 探测本机可用的 Java 列表。
///
/// 注意：`java -version` 把版本信息写到 stderr，因此两路输出都要读。
/// 单个候选失败直接跳过；一个都没找到时返回空 Vec。
pub fn detect_java() -> Result<Vec<JavaInfo>, AppError> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut found: Vec<JavaInfo> = Vec::new();

    // 先走环境变量（JAVA_HOME / PATH），再追加常见安装目录的兜底扫描。
    let mut candidates = java_candidates();
    for root in fallback_roots() {
        push_java_in_dir(&root, &mut candidates);
    }

    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }

        let path_str = candidate.to_string_lossy().to_string();
        let dedup_key = if cfg!(windows) {
            path_str.to_lowercase()
        } else {
            path_str.clone()
        };
        if !seen.insert(dedup_key) {
            continue;
        }

        let output = match Command::new(&candidate).arg("-version").output() {
            Ok(output) => output,
            Err(_) => continue,
        };

        let mut text = String::from_utf8_lossy(&output.stderr).to_string();
        text.push_str(&String::from_utf8_lossy(&output.stdout));

        let version = parse_java_version(&text).unwrap_or_else(|| "unknown".to_string());
        found.push(JavaInfo {
            path: path_str,
            version,
        });
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_detect_java() {
        let list = detect_java().unwrap();
        println!("detect_java() = {:#?}", list);
        println!("minecraft_dir() = {:?}", minecraft_dir());
        println!("launcher_config_path() = {:?}", launcher_config_path());
    }
}
