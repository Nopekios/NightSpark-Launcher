//! 实例文件校验：按版本 json 里的 SHA-1 核对 client jar 与 libraries。
//! 启动前先过一遍，把缺的、坏的列出来，比让用户对着「一启动就崩」猜有用。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::download::{self, VersionMeta};
use crate::error::AppError;
use crate::hash;
use crate::instance;

#[derive(Serialize, Clone, Debug, Default)]
pub struct VerifyReport {
    pub checked: usize,
    pub ok: usize,
    pub missing: Vec<String>,
    pub broken: Vec<String>,
    pub unverified: Vec<String>,
    /// 实例游戏目录里 mods 下的 jar 数（版本隔离的结构最容易在这里翻车）
    pub mods_jars: usize,
    pub notes: Vec<String>,
}

/// 校验实例：顺着 `inheritsFrom` 一路核到原版，父版本的文件同样要核。
pub fn verify_instance(instance_id: &str) -> Result<VerifyReport, AppError> {
    let instance = instance::get_instance(instance_id)?;
    let game_dir = PathBuf::from(&instance.game_dir);
    let mut report = VerifyReport::default();
    report.mods_jars = count_jars(&game_dir.join("mods"));
    if report.mods_jars == 0 {
        // 版本隔离的整合包内容留在 versions/<版本>/ 里，没搬进来就是纯净版
        report.notes.push(
            "实例的 mods 里一个 jar 都没有：整合包要是有版本隔离结构，先补一次本机版本文件".into(),
        );
    }

    let mut pending = vec![instance.version.clone()];
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(version) = pending.pop() {
        if !seen.insert(version.clone()) {
            continue;
        }
        let (meta, base) = match read_meta(&game_dir, &version) {
            Ok(value) => value,
            Err(err) => {
                report
                    .notes
                    .push(format!("{version} 的版本 json 读不出来：{err}"));
                continue;
            }
        };

        // client jar：清单一律写 sha1，缺了就算异常。
        if let Some(client) = &meta.downloads.client {
            let path = match &client.path {
                Some(relative) => base.join(relative),
                None => base.join(format!("{version}.jar")),
            };
            check_file(&path, client.sha1.as_deref(), &game_dir, &mut report);
        }

        for library in &meta.libraries {
            // 本地库是解压出来的，原 jar 不一定留，核了只会误报。
            if download::is_native_library(library) {
                continue;
            }
            if let Some(rules) = &library.rules {
                if !download::rules_allow(rules) {
                    continue;
                }
            }
            let Some(path) = download::library_file_path(library)? else {
                continue;
            };
            let sha1 = library
                .downloads
                .as_ref()
                .and_then(|downloads| downloads.artifact.as_ref())
                .and_then(|artifact| artifact.sha1.as_deref());
            check_file(&path, sha1, &game_dir, &mut report);
        }

        if let Some(parent) = meta.inherits_from {
            pending.push(parent);
        }
    }

    Ok(report)
}

pub(crate) fn count_jars(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("jar"))
                .unwrap_or(false)
        })
        .count()
}

/// 读版本 json：先看实例自己的 `versions/<ver>/`，没有再退回共享目录。
fn read_meta(game_dir: &Path, version: &str) -> Result<(VersionMeta, PathBuf), AppError> {
    let local_dir = game_dir.join("versions").join(version);
    let local_json = local_dir.join(format!("{version}.json"));
    let (text, base) = if local_json.exists() {
        (std::fs::read_to_string(&local_json)?, local_dir)
    } else {
        let shared_json = download::version_json_path(version)?;
        if !shared_json.exists() {
            return Err(AppError::NotFound(format!(
                "找不到 {version} 的版本 json"
            )));
        }
        (
            std::fs::read_to_string(&shared_json)?,
            download::version_dir(version)?,
        )
    };
    let meta: VersionMeta = serde_json::from_str(&text)
        .map_err(|err| AppError::Config(format!("版本 json 格式不对（{version}）：{err}")))?;
    Ok((meta, base))
}

/// 核对单个文件：不存在记 missing，没写 sha1 记 unverified，对不上记 broken。
fn check_file(path: &Path, sha1: Option<&str>, game_dir: &Path, report: &mut VerifyReport) {
    let label = path
        .strip_prefix(game_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    if !path.exists() {
        report.missing.push(label);
        return;
    }

    let Some(expected) = sha1.map(str::trim).filter(|value| !value.is_empty()) else {
        report.unverified.push(label);
        return;
    };

    report.checked += 1;
    match hash::sha1_file(path) {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => report.ok += 1,
        Ok(_) => report.broken.push(label),
        Err(err) => {
            report.broken.push(label);
            report.notes.push(format!("{} 读不出来：{err}", path.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_check_file_marks_missing_and_broken() {
        let dir = std::env::temp_dir().join("nightspark-verify-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录失败");

        let good = dir.join("good.txt");
        std::fs::write(&good, b"hello").expect("写文件失败");
        let good_sha1 = hash::sha1_file(&good).expect("算 sha1 失败");

        let broken = dir.join("broken.txt");
        std::fs::write(&broken, b"tampered").expect("写文件失败");

        let mut report = VerifyReport::default();
        check_file(&good, Some(&good_sha1), &dir, &mut report);
        check_file(&broken, Some(&good_sha1), &dir, &mut report);
        check_file(&dir.join("gone.jar"), Some(&good_sha1), &dir, &mut report);

        assert_eq!(report.checked, 2);
        assert_eq!(report.ok, 1);
        assert_eq!(report.broken, vec!["broken.txt".to_string()]);
        assert_eq!(report.missing, vec!["gone.jar".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
