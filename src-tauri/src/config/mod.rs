//! 启动器配置模块：路径解析、配置读写、本机 Java 探测。

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 静默跑子进程：启动器是 GUI 程序，探测 Java 之类的调用不加这个标志，
/// Windows 会给每个子进程弹一个黑色 CMD 窗口（设置页一次探测好几个 Java，就会连弹几个）。
fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LauncherConfig {
    /// Java 可执行文件路径，None 表示由启动器自动探测
    pub java_path: Option<String>,
    /// 分配给游戏的最大内存（MB）
    pub memory_mb: u32,
    /// 游戏目录覆盖值，None 表示使用默认 .minecraft
    pub game_dir: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            java_path: None,
            memory_mb: 4096,
            game_dir: None,
            language: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JavaInfo {
    pub path: String,
    pub version: String,
}

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

const CURRENT_DIR_NAME: &str = "NightSpark";
/// 项目改名前旧版启动器的文件夹名（`%APPDATA%\Love-Launcher`）。
const LEGACY_DIR_NAME: &str = "Love-Launcher";

/// 启动器自身配置文件路径：`config_dir/NightSpark/config.json`。
///
/// 刻意不放进 `.minecraft`，避免与游戏数据混在一起。
pub fn launcher_config_path() -> Result<PathBuf, AppError> {
    if let Some(home) = env_override("NIGHTSPARK_HOME") {
        return Ok(home.join("config.json"));
    }

    Ok(current_home_dir()?.join("config.json"))
}

/// 读取一个「非空即生效」的环境变量覆盖值，去掉首尾空白与成对引号。
///
/// 空串或只有空白一律视为未设置，避免命令行里留个 `NIGHTSPARK_HOME=` 就把路径解析带偏。
fn env_override(key: &str) -> Option<PathBuf> {
    let raw = env::var(key).ok()?;
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn current_home_dir() -> Result<PathBuf, AppError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 config 目录".to_string()))?;
    Ok(config_dir.join(CURRENT_DIR_NAME))
}

fn legacy_home_dir() -> Result<PathBuf, AppError> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 config 目录".to_string()))?;
    Ok(config_dir.join(LEGACY_DIR_NAME))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOutcome {
    Skipped,
    Renamed,
    Copied,
    Failed,
}

/// 把旧版 `%APPDATA%\Love-Launcher` 一次性迁移到 `%APPDATA%\NightSpark`。
///
/// **触发条件（三条同时满足才会真正搬运）**：
/// 1. 未设置 `NIGHTSPARK_HOME`。该变量一旦存在，配置根目录就由用户显式接管，
///    `%APPDATA%` 下的目录不再是权威数据，此时迁移既无意义、也可能动到不该动的目录；
/// 2. 旧目录 `Love-Launcher` 存在且确实是目录；
/// 3. 新目录 `NightSpark` **完全不存在**。
///
/// 第 3 条是「绝不覆盖已有数据」的关键约束。新目录只要存在，哪怕里面只有一个空文件，
/// 也说明这台机器已经用 NightSpark 跑过，配置、账号、实例都可能是比旧目录更新的版本。
/// 这种情况下既不做覆盖、也不做目录合并——两份 `instances` 合并会让同名实例互相污染，
/// 收益远低于风险，因此直接放弃迁移，交由用户自行处理。
///
/// **失败处理**：任何一步出错都只写一行 stderr 日志并返回 [`MigrationOutcome::Failed`]，
/// 不 panic、也不向上传递错误，保证旧目录状态异常时启动器照样能打开。
///
/// **旧目录去留**：
/// - 走 rename 成功时，旧目录已随改名消失，不存在清理问题；
/// - rename 失败（跨卷、被其他进程占用等）会退化为复制。复制成功后**保留**旧目录：
///   此时旧目录是唯一能证明「哪些文件原本就在」的副本，一旦复制有疏漏，自动删除
///   就会造成不可恢复的数据丢失。确认新目录可用后由用户手动删除更安全。
pub fn migrate_legacy_home() -> MigrationOutcome {
    if env_override("NIGHTSPARK_HOME").is_some() {
        log_migration("已设置 NIGHTSPARK_HOME，配置目录由用户接管，跳过");
        return MigrationOutcome::Skipped;
    }

    let current = match current_home_dir() {
        Ok(dir) => dir,
        Err(err) => {
            log_migration(&format!("无法定位当前配置目录，跳过: {err}"));
            return MigrationOutcome::Failed;
        }
    };

    let legacy = match legacy_home_dir() {
        Ok(dir) => dir,
        Err(err) => {
            log_migration(&format!("无法定位旧配置目录，跳过: {err}"));
            return MigrationOutcome::Failed;
        }
    };

    if !legacy.is_dir() {
        // 绝大多数情况走这里：已经迁移过或本来就是全新安装，不打日志避免每次启动刷屏。
        return MigrationOutcome::Skipped;
    }

    if current.exists() {
        log_migration(&format!(
            "新目录已存在（{}），为保护现有配置/账号/实例不做覆盖，跳过迁移",
            current.display()
        ));
        return MigrationOutcome::Skipped;
    }

    // 同一父目录下改名，等价于 mv，原子且不会产生半份数据。
    match std::fs::rename(&legacy, &current) {
        Ok(()) => {
            log_migration(&format!(
                "已将 {} 重命名为 {}",
                legacy.display(),
                current.display()
            ));
            MigrationOutcome::Renamed
        }
        Err(err) => {
            log_migration(&format!("直接改名失败（{err}），改用复制方式迁移"));
            match copy_dir_all(&legacy, &current) {
                Ok(files) => {
                    log_migration(&format!(
                        "已复制 {} 个文件到 {}，旧目录暂予保留，确认无误后可手动删除",
                        files,
                        current.display()
                    ));
                    MigrationOutcome::Copied
                }
                Err(err) => {
                    log_migration(&format!("复制迁移失败: {err}，清理半成品目录"));
                    // 半成品目录会让下次启动命中「新目录已存在」而永远不再重试，
                    // 这里只删除本次刚创建的目录，旧目录原样保留。
                    let _ = std::fs::remove_dir_all(&current);
                    MigrationOutcome::Failed
                }
            }
        }
    }
}

/// 递归复制目录，返回实际复制的文件数量；用于 rename 失败后的退化路径。
fn copy_dir_all(src: &Path, dst: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(dst).map_err(|err| format!("创建 {} 失败: {err}", dst.display()))?;

    let entries =
        std::fs::read_dir(src).map_err(|err| format!("读取 {} 失败: {err}", src.display()))?;

    let mut copied = 0usize;
    for entry in entries {
        let entry = entry.map_err(|err| format!("遍历 {} 失败: {err}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = entry
            .metadata()
            .map_err(|err| format!("读取 {} 属性失败: {err}", from.display()))?;

        if meta.is_dir() {
            copied += copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|err| format!("复制 {} 失败: {err}", from.display()))?;
            copied += 1;
        }
    }

    Ok(copied)
}

/// 迁移日志统一写 stderr：Windows 发布版没有控制台时不会阻塞启动，也不会污染前端。
fn log_migration(msg: &str) {
    eprintln!("[NightSpark] 配置目录迁移: {msg}");
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
    let mut cmd = Command::new(path);
    cmd.arg("-version");
    let output = quiet(&mut cmd).output().ok()?;
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

        let mut cmd = Command::new(&candidate);
        cmd.arg("-version");
        let output = match quiet(&mut cmd).output() {
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
