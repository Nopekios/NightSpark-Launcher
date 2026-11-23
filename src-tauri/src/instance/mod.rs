//! 实例管理模块：一个实例 = 游戏目录 + 版本 + 配置。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::AppError;

pub mod loader;
pub mod modpack;
pub mod verify;

const INSTANCE_FILE: &str = "instance.json";
const MAX_ID_RETRIES: u32 = 5;
const GAME_DIR_NAME: &str = ".minecraft";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub game_dir: String,
    pub version: String,
}

/// instances 根目录：`<launcher_config_dir>/instances`。只算路径，不创建。
fn instances_root() -> Result<PathBuf, AppError> {
    let config_path = config::launcher_config_path()?;
    let config_dir = config_path.parent().ok_or_else(|| {
        AppError::Config(format!("启动器配置路径没有父目录: {}", config_path.display()))
    })?;
    Ok(config_dir.join("instances"))
}

/// 把名字规整成 id 前缀：只保留 ASCII 字母数字（转小写），其余一律换成下划线，
/// 合并连续下划线并去掉首尾下划线；结果为空回退 `instance`。
fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;

    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !out.is_empty() && !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    let slug = out.trim_end_matches('_');
    if slug.is_empty() {
        "instance".to_string()
    } else {
        slug.to_string()
    }
}

/// 生成 4 位十六进制短后缀（xorshift 混合，无需额外依赖）。
fn short_suffix(seed: u64) -> String {
    let mut x = seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{:04x}", x & 0xffff)
}

fn generate_id(name: &str, attempt: u32) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let mixed = nanos ^ hasher.finish().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((attempt as u64) << 32);

    format!("{}_{}", slugify(name), short_suffix(mixed))
}

/// 读实例目录下的 `instance.json`；文件缺失或损坏都返回 Err，由调用方决定是否跳过。
fn read_instance(instance_dir: &Path) -> Result<Instance, AppError> {
    let path = instance_dir.join(INSTANCE_FILE);
    let raw = std::fs::read_to_string(&path)?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::Config(format!("解析实例文件失败（{}）: {}", path.display(), e)))
}

fn write_instance(instance_dir: &Path, instance: &Instance) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(instance)
        .map_err(|e| AppError::Config(format!("序列化实例失败: {}", e)))?;
    std::fs::write(instance_dir.join(INSTANCE_FILE), json)?;
    Ok(())
}

/// 列出全部实例。目录不存在返回空 Vec，坏实例跳过，按 name 排序。
pub fn list_instances() -> Result<Vec<Instance>, AppError> {
    let root = instances_root()?;

    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AppError::Io(e)),
    };

    let mut instances = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        match read_instance(&dir) {
            Ok(instance) => instances.push(instance),
            Err(_) => continue,
        }
    }

    instances.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(instances)
}

/// 按 id 取单个实例。不存在 -> NotFound，解析失败 -> Config。
pub fn get_instance(id: &str) -> Result<Instance, AppError> {
    let dir = instances_root()?.join(id);
    let path = dir.join(INSTANCE_FILE);

    if !path.exists() {
        return Err(AppError::NotFound(format!("实例不存在: {}", id)));
    }

    read_instance(&dir)
}

/// 把实例切到另一个版本 id（装完加载器后指向带加载器的版本）。
pub fn set_version(id: &str, version: &str) -> Result<Instance, AppError> {
    let version = version.trim();
    if version.is_empty() {
        return Err(AppError::Config("版本号不能为空".into()));
    }
    let dir = instances_root()?.join(id);
    let mut instance = read_instance(&dir)?;
    if instance.version == version {
        return Ok(instance);
    }
    instance.version = version.to_string();
    write_instance(&dir, &instance)?;
    Ok(instance)
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct CarryReport {
    pub files: usize,
    pub mods: usize,
    pub linked: usize,
    pub copied: usize,
    pub kept: usize,
    pub warnings: Vec<String>,
}

/// 版本目录里不该搬的：版本自身 jar/json、natives、日志与下载缓存。
fn is_isolation_skip(name: &str, version: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if name == format!("{version}.jar") || name == format!("{version}.json") {
        return true;
    }
    matches!(
        lower.as_str(),
        "natives"
            | "logs"
            | "crash-reports"
            | "cache"
            | ".mcsaves"
            | "manifest.json"
            | "profile.json"
            | "launcher_profiles.json"
    ) || lower.ends_with("-natives")
}

/// 把版本隔离目录里的游戏内容搬进实例。
///
/// PCL 的版本隔离把 mods / config / saves / options.txt 全塞在 `versions/<版本>/` 里，
/// 实例自己的 `.minecraft` 是空的——不搬过去启动出来就是纯净原版。
pub fn carry_version_files(
    game_dir: &Path,
    version_dir: &Path,
    version: &str,
    overwrite: bool,
) -> Result<CarryReport, AppError> {
    let mut report = CarryReport::default();
    if !version_dir.is_dir() {
        report
            .warnings
            .push(format!("本机没有 {} 的版本目录，没东西可搬", version_dir.display()));
        return Ok(report);
    }
    std::fs::create_dir_all(game_dir)?;
    for entry in std::fs::read_dir(version_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_isolation_skip(&name, version) {
            continue;
        }
        carry_entry(
            &entry.path(),
            &game_dir.join(&name),
            name.eq_ignore_ascii_case("mods"),
            overwrite,
            &mut report,
        )?;
    }
    Ok(report)
}

fn carry_entry(
    src: &Path,
    dst: &Path,
    in_mods: bool,
    overwrite: bool,
    report: &mut CarryReport,
) -> Result<(), AppError> {
    let meta = std::fs::symlink_metadata(src)?;
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let target = dst.join(entry.file_name());
            carry_entry(&entry.path(), &target, in_mods, overwrite, report)?;
        }
        return Ok(());
    }
    if !meta.is_file() {
        return Ok(());
    }
    if dst.exists() {
        if !overwrite {
            report.kept += 1;
            return Ok(());
        }
        std::fs::remove_file(dst)?;
    }
    // mods 体积大，同盘用硬链接省空间；其余（config、存档）会写盘，直接复制免得两边互相影响
    if in_mods {
        match std::fs::hard_link(src, dst) {
            Ok(()) => {
                report.linked += 1;
                report.files += 1;
                if src.extension().map(|ext| ext.eq_ignore_ascii_case("jar")).unwrap_or(false) {
                    report.mods += 1;
                }
                return Ok(());
            }
            Err(_) => {
                std::fs::copy(src, dst)?;
                report.copied += 1;
                report.files += 1;
                if src.extension().map(|ext| ext.eq_ignore_ascii_case("jar")).unwrap_or(false) {
                    report.mods += 1;
                }
                return Ok(());
            }
        }
    }
    std::fs::copy(src, dst)?;
    report.copied += 1;
    report.files += 1;
    let _ = name;
    Ok(())
}

pub fn sync_instance_from_shared(instance_id: &str) -> Result<CarryReport, AppError> {
    let instance = get_instance(instance_id)?;
    let version_dir = crate::download::version_dir(&instance.version)?;
    let game_dir = PathBuf::from(&instance.game_dir);
    carry_version_files(&game_dir, &version_dir, &instance.version, false)
}

#[derive(Serialize, Clone, Debug)]
pub struct ImportedInstance {
    pub instance: Instance,
    pub files: usize,
    pub mods: usize,
    pub reused: bool,
}

pub fn create_instance(name: &str, version: &str) -> Result<Instance, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Config("实例名称不能为空".into()));
    }
    let version = version.trim();

    let root = instances_root()?;

    let mut picked: Option<(String, PathBuf)> = None;
    for attempt in 0..MAX_ID_RETRIES {
        let id = generate_id(name, attempt);
        let dir = root.join(&id);
        if !dir.exists() {
            picked = Some((id, dir));
            break;
        }
    }
    let (id, instance_dir) =
        picked.ok_or_else(|| AppError::Internal("生成实例 ID 连续冲突 5 次，创建失败".into()))?;

    let game_dir = instance_dir.join(GAME_DIR_NAME);
    std::fs::create_dir_all(&game_dir)?;

    let instance = Instance {
        id,
        name: name.to_string(),
        game_dir: game_dir.to_string_lossy().into_owned(),
        version: version.to_string(),
    };
    write_instance(&instance_dir, &instance)?;

    Ok(instance)
}

pub fn delete_instance(id: &str) -> Result<(), AppError> {
    get_instance(id)?;

    let instance_dir = instances_root()?.join(id);
    std::fs::remove_dir_all(&instance_dir)?;
    Ok(())
}

fn scan_installed_versions() -> Result<Vec<String>, AppError> {
    let versions_dir = crate::download::shared_root()?.join("versions");
    if !versions_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&versions_dir)? {
        let entry = entry?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let version = entry.file_name().to_string_lossy().to_string();
        if version.starts_with('.') || !entry.path().join(format!("{version}.json")).is_file() {
            continue;
        }
        found.push(version);
    }
    found.sort();
    Ok(found)
}

/// 扫描共享游戏目录下已安装的版本，登记为实例。
///
/// 版本目录里被版本隔离出来的 mods / config / saves 一并搬进实例游戏目录——
/// 只登记版本不搬文件，启动出来就是纯净原版，等于白导。
/// 已经存在的实例也会再补一遍（老版本导进来的空壳实例靠这个救回来）。
pub fn import_installed_versions() -> Result<Vec<ImportedInstance>, AppError> {
    let existing = list_instances()?;
    let mut out = Vec::new();
    for version in scan_installed_versions()? {
        let Ok(version_dir) = crate::download::version_dir(&version) else {
            continue;
        };
        if let Some(found) = existing.iter().find(|item| item.version == version) {
            let carried =
                carry_version_files(&PathBuf::from(&found.game_dir), &version_dir, &version, false)?;
            out.push(ImportedInstance {
                instance: found.clone(),
                files: carried.files,
                mods: carried.mods,
                reused: true,
            });
            continue;
        }
        let instance = create_instance(&format!("本地 {version}"), &version)?;
        let carried =
            carry_version_files(&PathBuf::from(&instance.game_dir), &version_dir, &version, false)?;
        out.push(ImportedInstance {
            instance,
            files: carried.files,
            mods: carried.mods,
            reused: false,
        });
    }
    Ok(out)
}

pub fn import_modpack(path: &str) -> Result<modpack::ImportReport, AppError> {
    modpack::import_modpack(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_slugify() {
        assert_eq!(slugify("My Cool Pack!"), "my_cool_pack");
        // 纯中文没有任何 ASCII 字母数字 -> 回退 instance
        assert_eq!(slugify("测试实例"), "instance");
        assert_eq!(slugify("___"), "instance");
        assert_eq!(slugify("a*b/c"), "a_b_c");
        // 中文干扰项被替换掉，ASCII 部分保留并转小写
        assert_eq!(slugify("中文A1实例"), "a1");
        assert_eq!(slugify("  Steve  的 存档  "), "steve");
    }

    #[test]
    fn probe_create_instance_rejects_empty_name() {
        let err = create_instance("   ", "1.20.1").unwrap_err();
        assert!(matches!(err, AppError::Config(_)), "实际: {:?}", err);
    }

    #[test]
    fn probe_scan_installed_versions() {
        let list = scan_installed_versions().expect("scan_installed_versions 失败");
        println!("SCANNED({}) = {:?}", list.len(), list);
        assert!(list.iter().all(|v| !v.starts_with('.')), "不应包含隐藏目录");
        assert!(
            list.iter().all(|v| !v.trim().is_empty()),
            "不应出现空版本名"
        );
    }

    #[test]
    fn probe_instance_crud() {
        let name = "测试实例";
        let created = create_instance(name, "1.20.1").expect("create_instance 失败");
        println!("created = {:#?}", created);
        assert_eq!(created.name, name);
        assert_eq!(created.version, "1.20.1");

        let dir = instances_root().unwrap().join(&created.id);
        assert!(dir.is_dir(), "实例目录未创建");
        assert!(dir.join(INSTANCE_FILE).is_file(), "instance.json 未写入");
        assert!(dir.join(GAME_DIR_NAME).is_dir(), ".minecraft 未创建");
        assert_eq!(created.game_dir, dir.join(GAME_DIR_NAME).to_string_lossy());

        let all = list_instances().unwrap();
        println!("after create -> {} 条", all.len());
        for it in &all {
            println!("  {} | {} | {}", it.id, it.name, it.version);
        }
        assert!(all.iter().any(|it| it.id == created.id));

        let fetched = get_instance(&created.id).unwrap();
        assert_eq!(fetched.id, created.id);

        delete_instance(&created.id).unwrap();
        let err = get_instance(&created.id).unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "实际: {:?}", err);

        let all = list_instances().unwrap();
        println!("after delete -> {} 条", all.len());
        assert!(!all.iter().any(|it| it.id == created.id));
    }
}
