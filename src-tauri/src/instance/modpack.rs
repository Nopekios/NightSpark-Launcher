//! 整合包导入：识别 PCL / HMCL / 通用压缩包结构，解压进实例独立目录并登记版本。

use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::ZipArchive;

use crate::download;
use crate::error::AppError;

use super::{create_instance, Instance};

#[derive(Serialize, Clone, Debug)]
pub struct ImportReport {
    pub instance: Instance,
    pub kind: String,
    pub files: usize,
    /// 整合包要求的加载器写法（如 forge-47.2.0），纯原版整合包为 None
    pub loader: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingMod {
    /// 目标相对路径，例如 mods/xxx.jar
    pub path: String,
    /// 可直链下载时才有值（Modrinth 会给 cdn 链接，CurseForge 只给数字 ID）
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub source: String,
    /// 拿不到直链时告诉用户缺什么
    pub hint: Option<String>,
    /// CurseForge 的文件 ID：下载时拿它去查真名、直链和 SHA-1
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    /// 整合包是否把它标成非必装（manifest 里 required:false）。
    /// 老清单没有这个字段，缺省按必装处理，补齐失败时才会被当成硬伤。
    #[serde(default)]
    pub optional: bool,
}

#[derive(Serialize, Clone, Debug)]
struct PackPlan {
    kind: String,
    /// 要解压的前缀，多个前缀按顺序写入同一目标目录（后写的覆盖先写的）
    bases: Vec<String>,
    name: String,
    version: String,
    loader: Option<String>,
    pending: Vec<PendingMod>,
    warnings: Vec<String>,
}

pub fn import_modpack(path: &str) -> Result<ImportReport, AppError> {
    import_modpack_reported(path, &|_, _, _| {})
}

/// 带进度的导入：`progress(阶段, 已完成, 总数)` 会按「读取 → 解析 → 解压 → 收尾」上报。
pub fn import_modpack_reported(
    path: &str,
    progress: crate::download::ProgressFn<'_>,
) -> Result<ImportReport, AppError> {
    // 拖进来的可能不是 zip，而是本机已有的版本目录（PCL 版本隔离的 mods 全在版本目录里）
    if Path::new(path).is_dir() {
        return import_version_dir(path, progress);
    }

    progress("读取整合包", 0, 1);
    let file = File::open(path)
        .map_err(|err| AppError::NotFound(format!("打不开整合包 {path}：{err}")))?;
    let mut zip = ZipArchive::new(file)
        .map_err(|err| AppError::Config(format!("这不是有效的 zip 压缩包（{path}）：{err}")))?;
    progress("读取整合包", 1, 1);

    progress("扫描压缩包条目", 0, 1);
    let mut names = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|err| AppError::Config(format!("读取压缩包条目失败：{err}")))?;
        names.push(normalize(&decode_entry_name(entry.name(), entry.name_raw())));
    }
    progress("扫描压缩包条目", 1, 1);

    let fallback = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("导入的整合包")
        .to_string();

    progress("解析整合包结构", 0, 1);
    let plan = plan_for(&mut zip, &names, &fallback)?;
    let instance = create_instance(&plan.name, &plan.version)?;
    let game_dir = PathBuf::from(&instance.game_dir);
    progress("解析整合包结构", 1, 1);

    // 预统计要解压的条目数，进度条才有真实分母。
    let unpack_total = plan
        .bases
        .iter()
        .map(|base| names.iter().filter(|name| name.starts_with(base.as_str())).count() as u64)
        .sum::<u64>()
        .max(1);
    progress("解压整合包文件", 0, unpack_total);

    let mut files = 0usize;
    let mut base_done: u64 = 0;
    for base in &plan.bases {
        let offset = base_done;
        let extracted = extract_into(&mut zip, base, &game_dir, |n| {
            progress("解压整合包文件", offset + n, unpack_total);
        })?;
        files += extracted;
        base_done += extracted as u64;
    }
    progress("解压整合包文件", unpack_total, unpack_total);

    let mut warnings = plan.warnings;
    if files == 0 && plan.pending.is_empty() {
        warnings.push("压缩包里没有可解压的游戏文件，检查一下这个包是否完整".to_string());
    }

    if plan.pending.is_empty() {
        link_version(&game_dir, &plan.version, &mut warnings);
    } else {
        match write_pending(&game_dir, &plan.pending) {
            Ok(list) => {
                let required = plan.pending.iter().filter(|item| !item.optional).count();
                let optional = plan.pending.len() - required;
                let detail = if optional > 0 {
                    format!("还有 {required} 个必装 + {optional} 个非必装 mod 需要联网下载")
                } else {
                    format!("还有 {required} 个 mod 需要联网下载")
                };
                warnings.push(format!("包内只有配置，{detail}，清单已写入 {}", list.display()));
            }
            Err(err) => warnings.push(format!("待下载清单写入失败：{err}")),
        }
        if let Some(loader) = &plan.loader {
            warnings.push(format!(
                "{loader} 也要装，启动前确认已安装 {} 的加载器版本",
                plan.version
            ));
        }
    }

    // 把整合包要求的加载器记进实例目录，之后点「装加载器」不填版本也能装对。
    if let Some(loader) = plan.loader.as_deref() {
        if let Err(err) = write_pack_meta(&game_dir, loader, &plan.version) {
            warnings.push(format!("加载器信息没能落盘：{err}"));
        }
    }

    Ok(ImportReport {
        instance,
        loader: plan.loader.clone(),
        kind: plan.kind,
        files,
        warnings,
    })
}

/// 直接导入本机已有的版本目录。
///
/// PCL 的版本隔离把 mods / config / saves 全塞在 `versions/<版本>/` 下，
/// 实例自己的 .minecraft 是空的——所以这里必须把内容整份搬过去，否则启动就是原版。
fn import_version_dir(
    path: &str,
    progress: crate::download::ProgressFn<'_>,
) -> Result<ImportReport, AppError> {
    progress("读取本机版本", 0, 1);
    let dir = PathBuf::from(path);
    let version = resolve_version_dir(&dir)?;
    let instance = create_instance(&version, &version)?;
    progress("读取本机版本", 1, 1);
    progress("搬运版本隔离内容", 0, 1);
    let carried = super::carry_version_files(&PathBuf::from(&instance.game_dir), &dir, &version, false)?;
    progress("搬运版本隔离内容", 1, 1);
    let mut warnings = carried.warnings;
    if carried.mods == 0 {
        warnings.push("这个版本目录下没有 mods，导入进去只会是纯净版".to_string());
    }
    Ok(ImportReport {
        instance,
        kind: "本机版本".to_string(),
        files: carried.files,
        loader: None,
        warnings,
    })
}

/// 认一个目录是不是版本目录：自己带 `<目录名>.json` 就算；
/// 否则往下找 `versions/`，只认唯一的候选，多个就报错让用户自己挑。
fn resolve_version_dir(dir: &Path) -> Result<String, AppError> {
    if !dir.is_dir() {
        return Err(AppError::NotFound(format!("找不到目录 {}", dir.display())));
    }
    let own = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if !own.is_empty() && dir.join(format!("{own}.json")).is_file() {
        return Ok(own);
    }
    let versions = dir.join("versions");
    let mut candidates = Vec::new();
    if versions.is_dir() {
        for entry in std::fs::read_dir(&versions)? {
            let entry = entry?;
            let candidate = entry.file_name().to_string_lossy().to_string();
            if candidate.starts_with('.') {
                continue;
            }
            if entry.path().join(format!("{candidate}.json")).is_file() {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => Err(AppError::Config(format!(
            "{} 里没有 <版本>.json，请选 .minecraft\\versions 下具体的版本文件夹（比如「打金无限Final_加强版（整合包文件）」那个）",
            dir.display()
        ))),
        1 => Ok(candidates.remove(0)),
        _ => Err(AppError::Config(format!(
            "这个目录下有 {} 个版本，拖的时候请选具体的版本文件夹：{}",
            candidates.len(),
            candidates.join(" / ")
        ))),
    }
}

fn plan_for<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    names: &[String],
    fallback: &str,
) -> Result<PackPlan, AppError> {
    if let Some(base) = locate(names, "modpack.json") {
        let manifest = read_json(zip, &format!("{base}modpack.json"))?;
        let name = manifest
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback)
            .to_string();
        let version = manifest
            .get("gameVersion")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_string)
            .or_else(|| {
                detect_versions(names)
                    .filter(|(found, _)| *found == base)
                    .map(|(_, version)| version)
            })
            .ok_or_else(|| {
                AppError::Config("HMCL 整合包的 modpack.json 里没有 gameVersion，判断不出游戏版本".into())
            })?;
        return Ok(PackPlan {
            kind: "hmcl".to_string(),
            bases: vec![format!("{base}minecraft/")],
            name,
            version,
            loader: None,
            pending: Vec::new(),
            warnings: Vec::new(),
        });
    }

    if let Some(base) = locate(names, "modrinth.index.json") {
        let manifest = read_json(zip, &format!("{base}modrinth.index.json"))?;
        return parse_modrinth(&manifest, names, fallback);
    }

    if let Some(base) = locate(names, "manifest.json") {
        let manifest = read_json(zip, &format!("{base}manifest.json"))?;
        if manifest.get("addons").is_some() && manifest.get("manifestType").is_none() {
            return Err(AppError::Config(
                "这是 MCBBS 格式的整合包，它依赖已经关停的 MCBBS 接口，暂时装不了，建议找作者要 PCL 版".into(),
            ));
        }
        return parse_curseforge(&manifest, fallback, &base);
    }

    if let Some(base) = locate(names, "mmc-pack.json") {
        let manifest = read_json(zip, &format!("{base}mmc-pack.json"))?;
        return parse_mmc(&manifest, names, fallback, &base);
    }

    if locate(names, "mcbbs.packmeta").is_some() {
        return Err(AppError::Config(
            "这是 MCBBS 格式的整合包，接口已关停，暂时装不了，建议找作者要 PCL 版".into(),
        ));
    }

    if locate(names, "modpack.zip").is_some() || locate(names, "modpack.mrpack").is_some() {
        return Err(AppError::Config(
            "这个包里又套了一层整合包，先手动解压出内层压缩包再导入".into(),
        ));
    }

    if let Some((base, version)) = detect_versions(names) {
        return Ok(PackPlan {
            kind: "pcl".to_string(),
            bases: vec![base],
            name: fallback.to_string(),
            version,
            loader: None,
            pending: Vec::new(),
            warnings: Vec::new(),
        });
    }

    Err(AppError::Config(
        "没在压缩包里找到 versions\\<版本>\\<版本>.json 或任何已知的整合包清单，判断不出这是哪种整合包".into(),
    ))
}

fn parse_modrinth(
    manifest: &Value,
    names: &[String],
    fallback: &str,
) -> Result<PackPlan, AppError> {
    let deps = manifest.get("dependencies");
    let version = deps
        .and_then(|deps| deps.get("minecraft"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::Config("Modrinth 整合包的 dependencies 里没有 minecraft 版本".into())
        })?;

    let loader = ["neoforge", "forge", "fabric-loader", "quilt-loader"]
        .iter()
        .find_map(|key| {
            deps.and_then(|deps| deps.get(*key))
                .and_then(Value::as_str)
                .map(|value| format!("{key} {value}"))
        });

    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string();

    let mut pending = Vec::new();
    if let Some(files) = manifest.get("files").and_then(Value::as_array) {
        for file in files {
            // 只给服务端的 mod 客户端装了也白装
            if file
                .get("env")
                .and_then(|env| env.get("client"))
                .and_then(Value::as_str)
                == Some("unsupported")
            {
                continue;
            }
            let Some(path) = file.get("path").and_then(Value::as_str) else {
                continue;
            };
            let url = file
                .get("downloads")
                .and_then(Value::as_array)
                .and_then(|list| list.first())
                .and_then(Value::as_str)
                .map(str::to_string);
            let sha1 = file
                .get("hashes")
                .and_then(|hashes| hashes.get("sha1"))
                .and_then(Value::as_str)
                .map(str::to_string);
            pending.push(PendingMod {
                path: path.to_string(),
                url,
                sha1,
                source: "modrinth".to_string(),
                hint: None,
                file_id: None,
                optional: false,
            });
        }
    }

    let mut bases = Vec::new();
    if names.iter().any(|name| name.starts_with("overrides/")) {
        bases.push("overrides/".to_string());
    }
    if names.iter().any(|name| name.starts_with("client-overrides/")) {
        bases.push("client-overrides/".to_string());
    }

    Ok(PackPlan {
        kind: "modrinth".to_string(),
        bases,
        name,
        version,
        loader,
        pending,
        warnings: Vec::new(),
    })
}

fn parse_curseforge(
    manifest: &Value,
    fallback: &str,
    base: &str,
) -> Result<PackPlan, AppError> {
    let minecraft = manifest.get("minecraft");
    let version = minecraft
        .and_then(|node| node.get("version"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::Config("CurseForge 整合包的 minecraft.version 缺失，判断不出游戏版本".into())
        })?;

    let loader = minecraft
        .and_then(|node| node.get("modLoaders"))
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|item| {
                    item.get("primary")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .or_else(|| list.first())
        })
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string();

    let overrides = manifest
        .get("overrides")
        .and_then(Value::as_str)
        .unwrap_or("overrides");
    let override_base = format!("{base}{}/", overrides.trim_end_matches('/'));

    let mut pending = Vec::new();
    let mut warnings = Vec::new();
    if let Some(files) = manifest.get("files").and_then(Value::as_array) {
        for (index, file) in files.iter().enumerate() {
            // 非必装项（required:false）不能丢：很多包把「作者推荐但你没勾」的 mod
            // 也写进 files，直接跳过会让玩家进游戏才发现缺依赖。照收进清单，
            // 只打上可选标记，补齐环节据此分级提示。
            let optional = !file
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let project = file.get("projectID").and_then(Value::as_i64);
            let file_id = file.get("fileID").and_then(Value::as_i64);
            let (Some(project), Some(file_id)) = (project, file_id) else {
                // 缺标识就查不到直链：必装项得让用户知道补不上，非必装项不值得刷屏。
                if !optional {
                    warnings.push(format!(
                        "整合包清单第 {} 项缺 projectID / fileID，补装不了这个必装 mod，进游戏可能报缺 mod",
                        index + 1
                    ));
                }
                continue;
            };
            let hint = if optional {
                format!("projectID={project} fileID={file_id}，整合包标记为非必装")
            } else {
                format!("projectID={project} fileID={file_id}")
            };
            pending.push(PendingMod {
                path: format!("mods/{project}-{file_id}.jar"),
                url: None,
                sha1: None,
                source: "curseforge".to_string(),
                hint: Some(hint),
                file_id: Some(file_id),
                optional,
            });
        }
    }

    Ok(PackPlan {
        kind: "curseforge".to_string(),
        bases: vec![override_base],
        name,
        version,
        loader,
        pending,
        warnings,
    })
}

fn parse_mmc(
    manifest: &Value,
    names: &[String],
    fallback: &str,
    base: &str,
) -> Result<PackPlan, AppError> {
    let components = manifest
        .get("components")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut version = None;
    let mut loader = None;
    for component in &components {
        let uid = component.get("uid").and_then(Value::as_str).unwrap_or("");
        let value = component
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if uid == "net.minecraft" {
            if !value.is_empty() {
                version = Some(value);
            }
            continue;
        }
        if loader.is_none()
            && matches!(
                uid,
                "net.minecraftforge"
                    | "net.neoforged"
                    | "net.fabricmc.fabric-loader"
                    | "org.quiltmc.quilt-loader"
            )
        {
            loader = Some(format!("{uid} {value}"));
        }
    }

    let version = version.ok_or_else(|| {
        AppError::Config("Prism / MultiMC 整合包的 mmc-pack.json 里没有 net.minecraft 组件".into())
    })?;

    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string();

    let nested = format!("{base}.minecraft/");
    let bases = if names.iter().any(|name| name.starts_with(&nested)) {
        vec![nested]
    } else {
        vec![base.to_string()]
    };

    Ok(PackPlan {
        kind: "prism".to_string(),
        bases,
        name,
        version,
        loader,
        pending: Vec::new(),
        warnings: Vec::new(),
    })
}


#[derive(Serialize, Clone, Debug, Default)]
pub struct PendingReport {
    pub downloaded: usize,
    pub skipped: usize,
    /// 失败总数（含可选项）
    pub failed: usize,
    /// 失败里属于整合包非必装项的数量：必装缺失才会导致进游戏报缺 mod
    pub optional_failed: usize,
    pub messages: Vec<String>,
}

/// 缺失项的告警前缀：必装缺失会直接让游戏弹 Missing Mods，可选项只是少个功能。
fn grade_prefix(optional: bool) -> &'static str {
    if optional {
        "【可选】"
    } else {
        "【必装】"
    }
}

/// 按 pending-mods.json 把缺失的 mod 补齐，跑完把没搞定的写回清单。
///
/// CurseForge 的清单里只有 projectID / fileID，文件名、直链、SHA-1 都得现查
/// （直链按官方 CDN 规则拼，查不到再走镜像）；查到的文件按 CF 给的 modules
/// 落到 mods / resourcepacks / saves / shaderpacks。下载全程带 SHA-1 校验，
/// 校验不过的文件直接丢掉，不会留成「看起来装好了」的坏文件。
#[allow(dead_code)]
pub async fn download_pending(instance_id: &str) -> Result<PendingReport, AppError> {
    download_pending_reported(instance_id, &|_, _, _| {}).await
}

/// 带进度的补齐：查文件信息、下载两部分都会往 `progress` 上报。
pub async fn download_pending_reported(
    instance_id: &str,
    progress: crate::download::ProgressFn<'_>,
) -> Result<PendingReport, AppError> {
    let instance = super::get_instance(instance_id)?;
    let game_dir = PathBuf::from(&instance.game_dir);
    let list_path = game_dir.join(".nightspark").join("pending-mods.json");
    if !list_path.is_file() {
        return Err(AppError::NotFound(
            "这个实例没有待下载清单，它应该不是 CurseForge / Modrinth 整合包".into(),
        ));
    }

    let text = std::fs::read_to_string(&list_path)?;
    let payload: Value = serde_json::from_str(&text)
        .map_err(|err| AppError::Config(format!("待下载清单解析失败：{err}")))?;
    let mods = payload
        .get("mods")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // 清单是空的就没东西可补，直接把文件收掉，
    // 留着它只会让每次启动都白跑一遍补齐流程。
    if mods.is_empty() {
        let _ = std::fs::remove_file(&list_path);
        return Ok(PendingReport::default());
    }

    let mut report = PendingReport::default();
    let mut left: Vec<Value> = Vec::new();

    // 1) 先把 CurseForge 的 fileID 攒起来一次性批量查，省得一个文件一次请求。
    let mut cf_ids: Vec<i64> = Vec::new();
    for item in &mods {
        if item.get("source").and_then(Value::as_str) != Some("curseforge") {
            continue;
        }
        if let Some(id) = item.get("file_id").and_then(Value::as_i64) {
            if id > 0 && !cf_ids.contains(&id) {
                cf_ids.push(id);
            }
        }
    }
    let mut cf_files: std::collections::HashMap<i64, crate::net::CfFile> =
        std::collections::HashMap::new();
    if !cf_ids.is_empty() {
        progress("查询 CurseForge 文件信息", 0, 1);
        match crate::net::fetch_curseforge_files(&cf_ids).await {
            Ok(list) => {
                for file in list {
                    cf_files.insert(file.file_id, file);
                }
            }
            Err(err) => report
                .messages
                .push(format!("查 CurseForge 文件信息失败：{err}")),
        }
        progress("查询 CurseForge 文件信息", 1, 1);
    }

    // 2) 组装下载作业：每个文件都可能带多个候选源（官方 + 镜像）。
    let mut planned: Vec<(Value, download::FileJob)> = Vec::new();
    for item in mods {
        let optional = item
            .get("optional")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if path.is_empty() {
            report.failed += 1;
            if optional {
                report.optional_failed += 1;
            }
            report.messages.push(format!(
                "{}清单里有一项没有目标路径，跳过",
                grade_prefix(optional)
            ));
            continue;
        }
        let source = item
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut sha1 = item.get("sha1").and_then(Value::as_str).map(str::to_string);
        let mut urls: Vec<String> = Vec::new();
        if let Some(url) = item.get("url").and_then(Value::as_str) {
            if !url.trim().is_empty() {
                urls.push(url.to_string());
            }
        }
        let mut relative = path.clone();

        if source == "curseforge" {
            let file = item
                .get("file_id")
                .and_then(Value::as_i64)
                .and_then(|id| cf_files.get(&id));
            match file {
                Some(file) => {
                    // 作者禁了第三方分发时官方 downloadUrl 是空的，拼好的 CDN 地址兜底。
                    urls = file.urls.clone();
                    if sha1.is_none() {
                        sha1 = file.sha1.clone();
                    }
                    if !file.file_name.is_empty() {
                        relative = format!("{}/{}", file.folder, file.file_name);
                    }
                }
                None => {
                    report.failed += 1;
                    if optional {
                        report.optional_failed += 1;
                    }
                    let hint = item
                        .get("hint")
                        .and_then(Value::as_str)
                        .unwrap_or("CurseForge 没返回这个文件的信息");
                    report.messages.push(format!(
                        "{}{path} 拿不到下载直链（{hint}）",
                        grade_prefix(optional)
                    ));
                    left.push(item);
                    continue;
                }
            }
        }

        if urls.is_empty() {
            report.failed += 1;
            if optional {
                report.optional_failed += 1;
            }
            let hint = item
                .get("hint")
                .and_then(Value::as_str)
                .unwrap_or("清单里没有直链");
            report.messages.push(format!(
                "{}{path} 拿不到下载直链（{hint}）",
                grade_prefix(optional)
            ));
            left.push(item);
            continue;
        }

        planned.push((
            item,
            download::FileJob {
                urls,
                dest: game_dir.join(&relative),
                size: None,
                sha1,
            },
        ));
    }

    // 3) 并发下载；本地已有且哈希对得上的直接跳过。
    let (owners, jobs): (Vec<Value>, Vec<download::FileJob>) = planned.into_iter().unzip();
    let total = jobs.len() as u64;
    progress("下载缺失的 mod", 0, total);
    let summary = download::download_jobs(jobs, "下载缺失的 mod", progress).await?;
    progress("下载缺失的 mod", total, total);
    report.downloaded = summary.downloaded;
    report.skipped = summary.skipped;
    for (slot, error) in summary.failed_index.iter().zip(summary.failed.iter()) {
        let owner = owners.get(*slot);
        let optional = owner
            .and_then(|item| item.get("optional"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        report.failed += 1;
        if optional {
            report.optional_failed += 1;
        }
        report
            .messages
            .push(format!("{}{error}", grade_prefix(optional)));
        if let Some(owner) = owner {
            left.push(owner.clone());
        }
    }

    // 4) 没搞定的回写，下次点「补齐」接着来。
    // 全部搞定就把清单删掉，别留个空清单让下次启动再空转一遍
    if left.is_empty() {
        let _ = std::fs::remove_file(&list_path);
    } else {
        let remaining = serde_json::json!({"format": 1, "mods": left});
        let text = serde_json::to_string_pretty(&remaining)
            .map_err(|err| AppError::Config(format!("回写待下载清单失败：{err}")))?;
        std::fs::write(&list_path, text)?;
    }

    // 5) 有失败就给一条汇总：必装缺失会让游戏自己弹 Missing Mods 界面，
    //    得提前说清楚是「补齐没跑完」而不是整合包坏了。
    if report.failed > 0 {
        let required_failed = report.failed - report.optional_failed;
        report.messages.push(format!(
            "本次补齐失败 {} 个（必装 {} / 可选 {}）：必装缺失进游戏会弹 Missing Mods 界面，联网后重新补齐或手动下载；可选缺失只影响对应功能",
            report.failed, required_failed, report.optional_failed
        ));
    }

    // 6) 落一份带时间戳的最近一次报告，事后对账不用翻日志。
    if let Err(err) = write_pending_report(&game_dir, &report) {
        report.messages.push(format!("补齐报告没能落盘：{err}"));
    }

    Ok(report)
}

/// 把本次补齐结果写成 `<游戏目录>/.nightspark/last-pending-report.json`。
fn write_pending_report(game_dir: &Path, report: &PendingReport) -> Result<(), AppError> {
    let dir = game_dir.join(".nightspark");
    std::fs::create_dir_all(&dir)?;
    let payload = serde_json::json!({
        "format": 1,
        "generated_at": report_timestamp(),
        "failed": report.failed,
        "optional_failed": report.optional_failed,
        "downloaded": report.downloaded,
        "skipped": report.skipped,
        "messages": report.messages,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|err| AppError::Config(format!("生成补齐报告失败：{err}")))?;
    std::fs::write(dir.join("last-pending-report.json"), text)?;
    Ok(())
}

/// `2026-11-18T09:30:00Z` 形式的 UTC 时间戳；不为一行时间格式化再拉一个依赖。
fn report_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|gap| gap.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant 的 days-from-civil 逆运算：把纪元天数换成 (年, 月, 日)。
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let doe = (shifted - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 记录整合包要求的加载器，供「装加载器」在用户不填版本时使用。
fn write_pack_meta(game_dir: &Path, loader: &str, mc_version: &str) -> Result<(), AppError> {
    let dir = game_dir.join(".nightspark");
    std::fs::create_dir_all(&dir)?;
    let payload = serde_json::json!({
        "format": 1,
        "loader": loader,
        "mc_version": mc_version,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|err| AppError::Config(format!("生成整合包信息失败：{err}")))?;
    std::fs::write(dir.join("pack.json"), text)?;
    Ok(())
}

fn write_pending(game_dir: &Path, pending: &[PendingMod]) -> Result<PathBuf, AppError> {
    let dir = game_dir.join(".nightspark");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("pending-mods.json");
    let payload = serde_json::json!({
        "format": 1,
        "mods": pending,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|err| AppError::Config(format!("生成待下载清单失败：{err}")))?;
    std::fs::write(&path, text)?;
    Ok(path)
}

fn detect_versions(names: &[String]) -> Option<(String, String)> {
    for name in names {
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() < 3 {
            continue;
        }
        let file = parts[parts.len() - 1];
        let version = parts[parts.len() - 2];
        let marker = parts[parts.len() - 3];
        if marker != "versions" || file != format!("{version}.json").as_str() {
            continue;
        }
        let head = parts[..parts.len() - 3].join("/");
        let base = if head.is_empty() {
            String::new()
        } else {
            format!("{head}/")
        };
        return Some((base, version.to_string()));
    }
    None
}

fn locate(names: &[String], file: &str) -> Option<String> {
    for name in names {
        let Some(index) = name.rfind(file) else {
            continue;
        };
        if index + file.len() != name.len() {
            continue;
        }
        let head = &name[..index];
        if !head.is_empty() && !head.ends_with('/') {
            continue;
        }
        let prefix = head.trim_end_matches('/');
        if prefix.contains('/') {
            continue;
        }
        return Some(if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        });
    }
    None
}

fn read_json<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Value, AppError> {
    let mut raw = String::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| AppError::Config(format!("读取压缩包条目失败：{err}")))?;
        if normalize(&decode_entry_name(entry.name(), entry.name_raw())) != name {
            continue;
        }
        entry
            .read_to_string(&mut raw)
            .map_err(|err| AppError::Config(format!("读取 {name} 失败：{err}")))?;
        break;
    }
    if raw.is_empty() {
        return Err(AppError::Config(format!("压缩包里缺少 {name}")));
    }
    serde_json::from_str(&raw).map_err(|err| AppError::Config(format!("解析 {name} 失败：{err}")))
}

fn extract_into<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    base: &str,
    target: &Path,
    mut on_file: impl FnMut(u64),
) -> Result<usize, AppError> {
    let mut files = 0usize;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| AppError::Config(format!("读取压缩包条目失败：{err}")))?;
        let name = normalize(&decode_entry_name(entry.name(), entry.name_raw()));
        let Some(rel) = name.strip_prefix(base) else {
            continue;
        };
        if rel.is_empty() || rel == "instance.json" || rel.starts_with("__MACOSX/") {
            continue;
        }
        if rel.starts_with('/') || rel.contains(':') || rel.split('/').any(|seg| seg == "..") {
            continue;
        }

        let dest = target.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
        files += 1;
        on_file(files as u64);
    }
    Ok(files)
}

fn link_version(game_dir: &Path, version: &str, warnings: &mut Vec<String>) {
    let source = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    if !source.is_file() {
        let jar = game_dir
            .join("versions")
            .join(version)
            .join(format!("{version}.jar"));
        if !jar.is_file() {
            warnings.push(format!(
                "包里只带了 mods / config 一类的文件，没有 {version} 的版本 json，启动前得先把这个版本装好"
            ));
        }
        return;
    }

    let Ok(shared) = download::versions_dir() else {
        return;
    };
    let destination = shared.join(version);
    if destination.join(format!("{version}.json")).is_file() {
        return;
    }
    if let Err(err) = copy_dir(
        &game_dir.join("versions").join(version),
        &destination,
    ) {
        warnings.push(format!("把 {version} 登记到共享版本目录失败：{err}"));
    }
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 压缩包条目名：国内整合包大量用 GBK 编码且不带 UTF-8 标志，
/// 直接按 UTF-8 读会变乱码导致认不出 versions 目录，这里按需转码。
fn decode_entry_name(name: &str, raw: &[u8]) -> String {
    if std::str::from_utf8(raw).is_ok() {
        return name.to_string();
    }
    let (decoded, _, _) = encoding_rs::GBK.decode(raw);
    decoded.into_owned()
}

fn normalize(name: &str) -> String {
    let replaced = name.replace('\\', "/");
    let trimmed = replaced.trim_start_matches("./").trim_start_matches('/');
    trimmed.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这几个测试都要改进程级环境变量，得串行跑，否则家目录会串到别人的临时目录。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| normalize(item)).collect()
    }

    #[test]
    fn probe_detect_pcl_layout() {
        let entries = names(&[
            "整合包/versions/1.20.1-forge/1.20.1-forge.json",
            "整合包/mods/jei.jar",
        ]);
        assert_eq!(
            detect_versions(&entries),
            Some(("整合包/".to_string(), "1.20.1-forge".to_string()))
        );
    }

    #[test]
    fn probe_detect_top_level_layout() {
        let entries = names(&[".minecraft/versions/1.19.2/1.19.2.json"]);
        assert_eq!(
            detect_versions(&entries),
            Some((".minecraft/".to_string(), "1.19.2".to_string()))
        );
    }

    #[test]
    fn probe_locate_ignores_lookalike_name() {
        let entries = names(&["x/mymodpack.json"]);
        assert_eq!(locate(&entries, "modpack.json"), None);
        let entries = names(&["x/modpack.json"]);
        assert_eq!(locate(&entries, "modpack.json"), Some("x/".to_string()));
    }

    #[test]
    fn probe_locate_rejects_deep_nesting() {
        let entries = names(&["a/b/modpack.json"]);
        assert_eq!(locate(&entries, "modpack.json"), None);
    }

    #[test]
    fn probe_normalize_backslash() {
        assert_eq!(normalize("a\\b\\c.json"), "a/b/c.json");
        assert_eq!(normalize("./a/b/"), "a/b");
    }

    #[test]
    fn probe_import_reports_progress() {
        use std::io::Write;
        let _env = env_guard();

        let root = std::env::temp_dir().join("ll-modpack-progress");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let zip_path = root.join("进度整合包.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer
                .start_file("进度整合包/versions/1.20.1-forge/1.20.1-forge.json", opts)
                .unwrap();
            writer
                .write_all(br#"{"id":"1.20.1-forge","inheritsFrom":"1.20.1"}"#)
                .unwrap();
            writer.start_file("进度整合包/mods/jei.jar", opts).unwrap();
            writer.write_all(b"jar").unwrap();
            writer.start_file("进度整合包/config/jei.toml", opts).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }

        unsafe {
            std::env::set_var("NIGHTSPARK_HOME", root.join("home"));
            std::env::set_var("MINECRAFT_DIR", root.join("shared"));
        }

        let seen: std::sync::Mutex<Vec<(String, u64, u64)>> =
            std::sync::Mutex::new(Vec::new());
        let report = super::import_modpack_reported(zip_path.to_str().unwrap(), &|stage, done, total| {
            if let Ok(mut list) = seen.lock() {
                list.push((stage.to_string(), done, total));
            }
        })
        .expect("带进度导入失败");

        let seen = seen.lock().unwrap_or_else(|err| err.into_inner());
        let stages: Vec<String> = seen.iter().map(|(s, _, _)| s.clone()).collect();
        assert!(stages.iter().any(|s| s == "读取整合包"), "缺读取阶段：{stages:?}");
        assert!(stages.iter().any(|s| s == "解压整合包文件"), "缺解压阶段：{stages:?}");
        let extract: Vec<(u64, u64)> = seen
            .iter()
            .filter(|(s, _, _)| s == "解压整合包文件")
            .map(|(_, done, total)| (*done, *total))
            .collect();
        let last = extract.last().copied().unwrap_or((0, 0));
        assert!(
            last.0 == last.1 && last.1 >= 3,
            "解压进度没走满：{extract:?}"
        );
        assert!(report.files >= 3, "解压文件数偏少：{}", report.files);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_pcl_zip_end_to_end() {
        use std::io::Write;
        let _env = env_guard();

        let root = std::env::temp_dir().join("ll-modpack-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let zip_path = root.join("生电整合包.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer
                .start_file("生电整合包/versions/1.20.1-forge/1.20.1-forge.json", opts)
                .unwrap();
            writer
                .write_all(br#"{"id":"1.20.1-forge","inheritsFrom":"1.20.1"}"#)
                .unwrap();
            writer.start_file("生电整合包/mods/jei.jar", opts).unwrap();
            writer.write_all(b"jar").unwrap();
            writer.start_file("生电整合包/config/jei.toml", opts).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }

        // 把配置目录与共享游戏目录都关进临时根，别碰真实用户目录
        unsafe {
            std::env::set_var("NIGHTSPARK_HOME", root.join("home"));
            std::env::set_var("MINECRAFT_DIR", root.join("shared"));
        }

        let report = match super::import_modpack(zip_path.to_str().unwrap()) {
            Ok(report) => report,
            Err(err) => panic!("导入 PCL 整合包失败：{err:?}"),
        };

        assert_eq!(report.kind, "pcl");
        assert_eq!(report.instance.version, "1.20.1-forge");
        assert!(report.files >= 3, "解压文件数偏少：{}", report.files);

        let game_dir = PathBuf::from(&report.instance.game_dir);
        assert!(game_dir.join("mods/jei.jar").is_file());
        assert!(game_dir.join("config/jei.toml").is_file());
        assert!(game_dir
            .join("versions/1.20.1-forge/1.20.1-forge.json")
            .is_file());
        assert!(root
            .join("shared/versions/1.20.1-forge")
            .is_dir(), "版本没登记到共享 versions");

        let _ = std::fs::remove_dir_all(&root);
    }

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn probe_parse_modrinth_index() {
        let manifest = json(
            r#"{
              "formatVersion": 1,
              "name": "Fabulously Optimized",
              "versionId": "5.0",
              "dependencies": {"minecraft": "1.20.1", "fabric-loader": "0.15.0"},
              "files": [
                {"path": "mods/sodium.jar", "hashes": {"sha1": "aa"},
                 "downloads": ["https://cdn.modrinth.com/data/x/versions/y/sodium.jar"],
                 "env": {"client": "required"}},
                {"path": "mods/serveronly.jar", "hashes": {"sha1": "bb"},
                 "downloads": ["https://cdn.modrinth.com/z.jar"],
                 "env": {"client": "unsupported"}}
              ]
            }"#,
        );
        let entries = names(&["modrinth.index.json", "overrides/config/a.toml"]);
        let plan = parse_modrinth(&manifest, &entries, "fallback").unwrap();
        assert_eq!(plan.kind, "modrinth");
        assert_eq!(plan.version, "1.20.1");
        assert_eq!(plan.loader.as_deref(), Some("fabric-loader 0.15.0"));
        assert_eq!(plan.pending.len(), 1, "只给服务端的 mod 不该进清单");
        assert_eq!(plan.pending[0].path, "mods/sodium.jar");
        assert!(plan.pending[0].url.is_some());
        assert_eq!(plan.bases, vec!["overrides/".to_string()]);
    }

    #[test]
    fn probe_parse_curseforge_manifest() {
        let manifest = json(
            r#"{
              "minecraft": {"version": "1.19.2",
                            "modLoaders": [{"id": "forge-43.2.0", "primary": true}]},
              "manifestType": "minecraftModpack",
              "name": "All the Mods",
              "files": [
                {"projectID": 238222, "fileID": 4567, "required": true},
                {"projectID": 1, "fileID": 2, "required": false}
              ],
              "overrides": "overrides"
            }"#,
        );
        let plan = parse_curseforge(&manifest, "fallback", "").unwrap();
        assert_eq!(plan.kind, "curseforge");
        assert_eq!(plan.version, "1.19.2");
        assert_eq!(plan.loader.as_deref(), Some("forge-43.2.0"));
        assert_eq!(plan.pending.len(), 2, "必装项和非必装项都要进清单");
        assert!(plan.pending[0].url.is_none(), "CurseForge 只给数字 ID");
        assert!(!plan.pending[0].optional, "required:true 不该标成可选");
        assert!(plan.pending[1].optional, "required:false 应标成可选");
        assert_eq!(plan.bases, vec!["overrides/".to_string()]);
    }

    /// 非必装项不能像以前那样被 continue 丢掉，得带着 optional 标记留在清单里。
    #[test]
    fn parse_curseforge_keeps_optional_files() {
        let manifest = json(
            r#"{
              "minecraft": {"version": "1.20.1",
                            "modLoaders": [{"id": "forge-47.2.0", "primary": true}]},
              "manifestType": "minecraftModpack",
              "name": "带可选 mod 的包",
              "files": [
                {"projectID": 238222, "fileID": 4567, "required": true},
                {"projectID": 306612, "fileID": 8901, "required": false}
              ],
              "overrides": "overrides"
            }"#,
        );
        let plan = parse_curseforge(&manifest, "fallback", "").unwrap();
        assert_eq!(
            plan.pending.len(),
            2,
            "required:false 的文件被丢掉了：{:?}",
            plan.pending
        );

        let required = plan
            .pending
            .iter()
            .find(|item| item.file_id == Some(4567))
            .expect("必装项不见了");
        assert!(!required.optional, "required:true 应标成必装");
        assert_eq!(required.path, "mods/238222-4567.jar");

        let optional = plan
            .pending
            .iter()
            .find(|item| item.file_id == Some(8901))
            .expect("required:false 的文件没进清单");
        assert!(optional.optional, "required:false 应标成可选");
        assert_eq!(optional.path, "mods/306612-8901.jar");
        assert!(
            optional
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("非必装"),
            "可选项的 hint 没注明非必装：{:?}",
            optional.hint
        );

        // 序列化写清单时 optional 字段必须带上，否则补齐环节认不出可选项。
        let dumped = serde_json::to_value(optional).unwrap();
        assert_eq!(
            dumped.get("optional").and_then(Value::as_bool),
            Some(true),
            "optional 没序列化进清单：{dumped}"
        );
        // 老清单不带 optional 字段时按必装处理。
        let legacy: PendingMod = serde_json::from_str(
            r#"{"path":"mods/a.jar","url":null,"sha1":null,"source":"curseforge","hint":null}"#,
        )
        .unwrap();
        assert!(!legacy.optional, "缺 optional 字段的老清单应按必装处理");
    }

    #[test]
    fn probe_parse_prism_pack() {
        let manifest = json(
            r#"{"formatVersion": 1, "components": [
                 {"uid": "net.minecraft", "version": "1.20.1"},
                 {"uid": "net.minecraftforge", "version": "47.2.0"}
               ]}"#,
        );
        let entries = names(&[".minecraft/mods/a.jar", "mmc-pack.json"]);
        let plan = parse_mmc(&manifest, &entries, "fallback", "").unwrap();
        assert_eq!(plan.kind, "prism");
        assert_eq!(plan.version, "1.20.1");
        assert_eq!(plan.loader.as_deref(), Some("net.minecraftforge 47.2.0"));
        assert_eq!(plan.bases, vec![".minecraft/".to_string()]);
    }

    #[test]
    fn import_modrinth_zip_end_to_end() {
        use std::io::Write;
        let _env = env_guard();

        let root = std::env::temp_dir().join("ll-modpack-e2e-modrinth");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let zip_path = root.join("fabulously.mrpack");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file("modrinth.index.json", opts).unwrap();
            writer
                .write_all(
                    br#"{"formatVersion":1,"name":"FO","dependencies":{"minecraft":"1.20.1","fabric-loader":"0.15.0"},
                         "files":[{"path":"mods/sodium.jar","hashes":{"sha1":"aa"},
                                   "downloads":["https://cdn.modrinth.com/sodium.jar"]}]}"#,
                )
                .unwrap();
            writer
                .start_file("overrides/config/fo.toml", opts)
                .unwrap();
            writer.write_all(b"enabled = true").unwrap();
            writer.finish().unwrap();
        }

        unsafe {
            std::env::set_var("NIGHTSPARK_HOME", root.join("home"));
            std::env::set_var("MINECRAFT_DIR", root.join("shared"));
        }

        let report = super::import_modpack(zip_path.to_str().unwrap()).expect("Modrinth 导入失败");
        assert_eq!(report.kind, "modrinth");
        assert_eq!(report.instance.version, "1.20.1");

        let game_dir = PathBuf::from(&report.instance.game_dir);
        assert!(game_dir.join("config/fo.toml").is_file(), "overrides 没解压出来");
        let pending = game_dir.join(".nightspark/pending-mods.json");
        assert!(pending.is_file(), "待下载清单没写出来");
        let text = std::fs::read_to_string(&pending).unwrap();
        assert!(text.contains("sodium.jar"), "清单里没有 mod：{text}");
        assert!(
            report.warnings.iter().any(|line| line.contains("联网下载")),
            "缺一条需要联网下载的提示：{:?}",
            report.warnings
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
