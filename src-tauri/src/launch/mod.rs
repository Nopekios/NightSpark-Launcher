//! 启动模块：组装 JVM / 游戏参数并拉起 Java 进程。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::auth::Account;
use crate::download::{self, ArgValue, VersionMeta};
use crate::error::AppError;
use crate::instance::Instance;

/// 一次启动使用的参数配置。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LaunchConfig {
    pub java_path: String,
    pub memory_mb: u32,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    #[serde(default)]
    pub game_args: Vec<String>,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            java_path: String::new(),
            memory_mb: 4096,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
        }
    }
}

/// 启动结果。
#[derive(Debug, Clone, Serialize)]
pub struct LaunchOutcome {
    pub pid: u32,
    pub log_path: String,
    /// 启动器侧的 stdout/stderr 落盘位置，游戏还没写出 latest.log 时靠它看真实报错。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub console_log_path: String,
    /// 脱敏后的命令行摘要，仅供界面上展示。
    pub summary: String,
    /// 自动切换 Java 时给出的说明，没切换就是 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_note: Option<String>,
    /// 启动前自动补的东西，按顺序给界面提示用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prepare_notes: Vec<String>,
}

/// 把一条参数（可能是条件参数）按平台规则展开进 `out`。
fn push_arg(arg: &ArgValue, out: &mut Vec<String>, vars: &HashMap<&str, String>) {
    match arg {
        ArgValue::Plain(text) => out.push(download::substitute(text, vars)),
        ArgValue::Conditional { rules, value } => {
            if download::rules_allow(rules) {
                value.push_into(out, vars);
            }
        }
    }
}

/// 校验所选 Java 能不能跑这个版本。
///
/// 低版本 MC 对 Java 上限很敏感：1.16.5 及以下必须 Java 8，用 17 以上会直接
/// 崩在 LWJGL / 反射上；高版本则只要求不低于元数据里写的 Java 主版本。
fn ensure_java_matches(meta: &VersionMeta, java_path: &str) -> Result<(), AppError> {
    let Some(required) = meta.java_version.as_ref().map(|j| j.major_version) else {
        return Ok(());
    };
    // 探测不出来（非标准发行版）就不拦，交给游戏自己报错。
    let Some(actual) = crate::config::probe_major(java_path) else {
        return Ok(());
    };
    let mismatch = if required <= 8 {
        actual != required
    } else {
        actual < required
    };
    if mismatch {
        return Err(AppError::Config(format!(
            "{} 需要 Java {}，当前选的是 Java {}（{}）。请到设置里装一个 Java {}，或者切换到已有的 Java {} 再启动",
            meta.id, required, actual, java_path, required, required
        )));
    }
    Ok(())
}

/// 检查参数里有没有没被替换掉的 `${...}` 占位符。
///
/// 有就当场报错并指出变量名，比让 JVM 抛 InvalidPathException 好懂。
fn check_placeholders(args: &[String]) -> Result<(), AppError> {
    for arg in args {
        let mut rest = arg.as_str();
        while let Some(pos) = rest.find("${") {
            let after = &rest[pos + 2..];
            let Some(end) = after.find('}') else { break };
            let name = &after[..end];
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
            if valid {
                return Err(AppError::Config(format!(
                    "启动参数里的 ${{{name}}} 没有对应的值，启动器没提供这个变量；相关参数：{arg}"
                )));
            }
            rest = after;
        }
    }
    Ok(())
}

/// 组装启动命令，返回「命令 + 脱敏摘要」。
fn build_command(
    instance: &Instance,
    meta: &VersionMeta,
    account: &Account,
    cfg: &LaunchConfig,
) -> Result<(Command, String), AppError> {
    if cfg.java_path.trim().is_empty() {
        return Err(AppError::Config(
            "还没有选择 Java 运行环境，请先在设置里选一个".to_string(),
        ));
    }
    if !PathBuf::from(&cfg.java_path).is_file() {
        return Err(AppError::NotFound(format!(
            "Java 路径不存在: {}",
            cfg.java_path
        )));
    }
    if !download::is_installed(&instance.version) {
        return Err(AppError::NotFound(format!(
            "版本 {} 还没下载完，请先下载该版本",
            instance.version
        )));
    }

    let game_dir = PathBuf::from(&instance.game_dir);
    if download::shared_root().map(|root| root == game_dir).unwrap_or(false) {
        return Err(AppError::Config(
            "这个实例的游戏目录就是共享 .minecraft，mods 和配置会串到别的实例，请改用实例独立目录"
                .to_string(),
        ));
    }
    std::fs::create_dir_all(&game_dir)?;

    ensure_java_matches(meta, &cfg.java_path)?;

    let classpath_entries = download::collect_classpath(meta, &instance.version)?;
    if classpath_entries.is_empty() {
        return Err(AppError::NotFound(
            "classpath 为空，依赖库可能没有下载完整".to_string(),
        ));
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    let classpath = classpath_entries
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(separator);

    let natives_dir = download::natives_dir(&instance.version)?;
    let library_dir = download::libraries_dir()?;
    let assets_root = download::assets_dir()?;

    let token = account
        .access_token
        .clone()
        .ok_or_else(|| AppError::Config("当前账号没有 Minecraft 访问令牌，请重新登录".to_string()))?;

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", account.name.clone());
    vars.insert("version_name", meta.id.clone());
    vars.insert("game_directory", game_dir.to_string_lossy().to_string());
    vars.insert("assets_root", assets_root.to_string_lossy().to_string());
    vars.insert(
        "assets_index_name",
        meta.assets.clone().unwrap_or_else(|| "legacy".to_string()),
    );
    vars.insert("auth_uuid", account.uuid.clone());
    vars.insert("auth_access_token", token.clone());
    vars.insert("auth_session", format!("token:{token}"));
    vars.insert("user_type", "msa".to_string());
    vars.insert("user_properties", "{}".to_string());
    vars.insert("clientid", String::new());
    vars.insert("auth_xuid", String::new());
    vars.insert("version_type", meta.kind.clone().unwrap_or_else(|| "release".to_string()));
    vars.insert("launcher_name", "Love-Launcher".to_string());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").to_string());
    vars.insert("classpath", classpath.clone());
    vars.insert("natives_directory", natives_dir.to_string_lossy().to_string());
    vars.insert("library_directory", library_dir.to_string_lossy().to_string());
    // Forge 1.17+ / NeoForge 的 bootstraplauncher 用 -p 拼模块路径，里面写的是
    // ${classpath_separator}，缺了这个变量整串会被原样塞给 JVM 然后报
    // InvalidPathException: Illegal char <:>。
    vars.insert("classpath_separator", separator.to_string());
    // 老版本（1.6 及以前）用 game_assets，1.12- 用 resolution_* 与 user_data。
    vars.insert(
        "game_assets",
        assets_root.join("legacy").to_string_lossy().to_string(),
    );
    vars.insert("user_data", game_dir.to_string_lossy().to_string());
    vars.insert("resolution_width", "854".to_string());
    vars.insert("resolution_height", "480".to_string());

    let mut jvm_args: Vec<String> = Vec::new();
    jvm_args.push(format!("-Xmx{}M", cfg.memory_mb.max(512)));
    jvm_args.push(format!("-Xms{}M", (cfg.memory_mb / 2).max(256)));
    // natives 包被 collect_classpath 排除在 classpath 之外（不能当模块），
    // LWJGL 只能靠 java.library.path / org.lwjgl.librarypath 去找 lwjgl.dll。
    // 1.13+ 的版本 json 里没有这条参数，官方启动器是自己补的；漏了它
    // Sodium 在 ImmediateWindowHandler 里探测显卡时直接
    // UnsatisfiedLinkError: Failed to locate library: lwjgl.dll，进程静默秒退。
    let natives_str = natives_dir.to_string_lossy().to_string();
    jvm_args.push(format!("-Djava.library.path={natives_str}"));
    jvm_args.push(format!("-Dorg.lwjgl.librarypath={natives_str}"));

    let mut game_args: Vec<String> = Vec::new();
    match &meta.arguments {
        Some(arguments) => {
            for arg in &arguments.jvm {
                push_arg(arg, &mut jvm_args, &vars);
            }
            for arg in &arguments.game {
                push_arg(arg, &mut game_args, &vars);
            }
        }
        None => {
            // 1.12 及以前只有 minecraftArguments 字符串。
            jvm_args.push(format!(
                "-Djava.library.path={}",
                natives_dir.to_string_lossy()
            ));
            jvm_args.push("-cp".to_string());
            jvm_args.push(classpath.clone());
            if let Some(raw) = &meta.minecraft_arguments {
                for token in raw.split_whitespace() {
                    game_args.push(download::substitute(token, &vars));
                }
            }
        }
    }

    // 版本 json 若自带同名 -D 参数，只保留我们先补的那条，避免重复。
    for prefix in [
        "-Djava.library.path=",
        "-Dorg.lwjgl.librarypath=",
    ] {
        let mut seen = false;
        jvm_args.retain(|arg| {
            if arg.starts_with(prefix) {
                if seen {
                    return false;
                }
                seen = true;
            }
            true
        });
    }

    // 1.13+ 的版本元数据里 jvm 参数本来就不带 -cp，官方启动器是自己补的。
    // 少了它 java.class.path 会退回成 "."，BootStrapLauncher 读到的 legacyClassPath 就是空的，
    // 于是找不到 FML 注册的 java.util.function.Consumer 服务，直接 NoSuchElementException。
    let has_classpath = jvm_args.iter().any(|a| {
        a == "-cp"
            || a == "-classpath"
            || a.starts_with("-cp=")
            || a.starts_with("-classpath=")
    });
    if !has_classpath {
        jvm_args.push("-cp".to_string());
        jvm_args.push(classpath.clone());
    }

    // 用户自定义参数放在最后，覆盖前面生成的值。
    jvm_args.extend(cfg.jvm_args.iter().filter(|s| !s.trim().is_empty()).cloned());
    game_args.extend(cfg.game_args.iter().filter(|s| !s.trim().is_empty()).cloned());
    // ---- Forge / NeoForge 的 -DignoreList 修正 ----
    // 版本 json 里写的是 `-DignoreList=client-extra,${version_name}.jar`，其中的
    // ${version_name} 会被我们展开成加载器版本 id（如 neoforge-21.1.266），于是只
    // 忽略了加载器自己的 jar。可 classpath 上还有顺着 inheritsFrom 带上的原版 jar
    // （1.21.1.jar），它不在忽略名单里就会被 securejarhandler 当成自动模块塞进模块层
    // （模块名退化成一个 `_1._21._1`），和 FML 生成的 minecraft 模块重复导出
    // net.minecraft.client，最后抛 java.lang.module.ResolutionException。
    // 这个异常被 BootstrapLauncher 一层层包成 InvocationTargetException 打出来，日志里
    // 只剩一行被截断的 cause，看着像"无缘无故的崩溃"。
    // 修法：把继承来的原版 jar 一并补进 ignoreList（已有的不重复加），让它回到 legacy
    // classpath，vanilla 类统一由 minecraft 模块提供。
    if let Some(pos) = jvm_args.iter().position(|a| a.starts_with("-DignoreList=")) {
        let current = jvm_args[pos]
            .trim_start_matches("-DignoreList=")
            .to_string();
        let mut list: Vec<String> = current
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let mut wanted: Vec<String> = Vec::new();
        if let Some(parent) = meta.inherits_from.as_ref() {
            wanted.push(format!("{parent}.jar"));
        }
        wanted.push(format!("{}.jar", meta.id));
        for jar in wanted {
            if !list.iter().any(|x| x == &jar) {
                list.push(jar);
            }
        }
        jvm_args[pos] = format!("-DignoreList={}", list.join(","));
    }

    // 占位符没被替换掉的话，宁可在这里说清缺哪个变量，也不要让 JVM 抛一堆看不懂的堆栈。
    check_placeholders(&jvm_args)?;
    check_placeholders(&game_args)?;

    let mut command = Command::new(&cfg.java_path);
    command
        .args(&jvm_args)
        .arg(&meta.main_class)
        .args(&game_args)
        .current_dir(&game_dir);

    // 摘要里把令牌打码，避免日志/界面泄露。
    let mut summary_parts = vec![cfg.java_path.clone()];
    summary_parts.extend(jvm_args.iter().map(|a| {
        if a.contains(&token) {
            a.replace(&token, "***")
        } else {
            a.clone()
        }
    }));
    summary_parts.push(meta.main_class.clone());
    summary_parts.extend(game_args.iter().map(|a| {
        if a.contains(&token) {
            a.replace(&token, "***")
        } else {
            a.clone()
        }
    }));
    let summary = summary_parts.join(" ");

    Ok((command, summary))
}

/// 启动游戏，返回子进程 PID 与日志路径。`on_exit` 在游戏进程结束时于后台线程回调。
pub fn launch(
    instance: &Instance,
    meta: &VersionMeta,
    account: &Account,
    cfg: &LaunchConfig,
    on_exit: Option<Box<dyn FnOnce(u32) + Send>>,
) -> Result<LaunchOutcome, AppError> {
    let (mut command, summary) = build_command(instance, meta, account, cfg)?;

    let game_dir = PathBuf::from(&instance.game_dir);
    let logs_dir = game_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    // 注意：这里**不能**再往 latest.log 写。
    // latest.log 是游戏内 log4j 的专属文件，它以截断方式打开；启动器若同时持有同一个
    // 文件的句柄往里面推 stdout/stderr，两个文件指针会互相覆盖，结果就是日志只剩开头
    // 几行、最后的崩溃信息（往往只在 stderr 里）被吃掉，看起来像"没崩溃也没启动"。
    // 控制台输出单独落 launcher-console.log，完整保留命令行、JVM 报错与 native 崩溃。
    let console_path = logs_dir.join("launcher-console.log");
    let mut console_file = std::fs::File::create(&console_path)?;
    writeln!(console_file, "=== Love-Launcher 启动 ===")?;
    writeln!(console_file, "$ {summary}")?;
    writeln!(console_file, "==========================")?;
    let stderr_file = console_file.try_clone()?;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(console_file))
        .stderr(Stdio::from(stderr_file));

    #[cfg(windows)]
    {
        // 避免弹出黑色控制台窗口。
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|e| AppError::Internal(format!("启动 Java 失败: {e}")))?;
    let pid = child.id();

    // 进程退出后补一行退出码：Java 正常退出是 0，被强杀是 1/其他，native 崩溃常见
    // 0xC0000005 这类大数。没有这行，就只能靠猜"到底是没开还是崩了"。
    // on_exit 依然只在进程真正结束后才回调。
    let exit_log = console_path.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&exit_log) {
            let _ = writeln!(f, "=== 游戏进程已退出，退出码 {code}（pid {pid}）===");
        }
        if let Some(callback) = on_exit {
            callback(pid);
        }
    });

    Ok(LaunchOutcome {
        pid,
        log_path: logs_dir.join("latest.log").to_string_lossy().to_string(),
        console_log_path: console_path.to_string_lossy().to_string(),
        summary,
        java_note: None,
        prepare_notes: Vec::new(),
    })
}


#[cfg(test)]
mod live_tests {
    use super::*;

    /// 真机拉 Forge：确认 bootstraplauncher 的 ${classpath_separator} 已经换成本平台的
    /// 分隔符，不会再抛 InvalidPathException: Illegal char <:>。
    #[test]
    #[ignore]
    fn live_launch_forge_boot_layer() {
        let version = std::env::var("TEST_VERSION")
            .unwrap_or_else(|_| "1.20.1-forge-47.4.18".to_string());
        if !download::is_installed(&version) {
            println!("skip: {version} 还没安装");
            return;
        }
        let meta = download::load_merged_meta(&version).expect("读取版本元数据失败");
        let java = std::env::var("TEST_JAVA")
            .unwrap_or_else(|_| r"C:\Dev\JDK\jdk-21.0.13+9\bin\java.exe".to_string());
        let game_dir = std::env::temp_dir().join("love-launcher-forge-test");
        std::fs::create_dir_all(&game_dir).unwrap();
        let instance = Instance {
            id: "live-forge".to_string(),
            name: "live-forge".to_string(),
            game_dir: game_dir.to_string_lossy().to_string(),
            version: version.clone(),
        };
        let account = Account {
            uuid: "00000000000000000000000000000000".to_string(),
            name: "OfflineTester".to_string(),
            access_token: Some("fake-token-for-arg-check".to_string()),
            refresh_token: None,
            expires_at: None,
        };
        let cfg = LaunchConfig {
            java_path: java,
            memory_mb: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
        };
        let outcome = launch(&instance, &meta, &account, &cfg, None).expect("拉起 Forge 失败");
        println!("pid={} log={}", outcome.pid, outcome.log_path);
        println!("summary={}", outcome.summary);
        assert!(
            !outcome.summary.contains("${"),
            "命令行摘要里还有没替换的占位符：{}",
            outcome.summary
        );
        std::thread::sleep(std::time::Duration::from_secs(25));
        let log = std::fs::read_to_string(&outcome.log_path).unwrap_or_default();
        let head: String = log.chars().take(3000).collect();
        println!("---- log head ----\n{head}");
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &outcome.pid.to_string(), "/T", "/F"])
            .output();
        assert!(
            !log.contains("InvalidPathException"),
            "启动参数里还有没替换掉的占位符，日志：{head}"
        );
    }

    /// 回归：Forge/NeoForge 的 -DignoreList 必须把继承来的原版 jar 也列进去，
    /// 否则原版 jar 会作为自动模块（模块名退化成 `_1._21._1`）和 minecraft 模块
    /// 重复导出 net.minecraft.client，启动时只剩一行被截断的 InvocationTargetException。
    #[test]
    #[ignore]
    fn ignore_list_covers_parent_jar() {
        let version = std::env::var("TEST_VERSION")
            .unwrap_or_else(|_| "neoforge-21.1.266".to_string());
        if !download::is_installed(&version) {
            println!("skip: {version} 还没安装");
            return;
        }
        let meta = download::load_merged_meta(&version).expect("读取版本元数据失败");
        let java = std::env::var("TEST_JAVA")
            .unwrap_or_else(|_| r"C:\Dev\JDK\jdk-21.0.13+9\bin\java.exe".to_string());
        let instance = Instance {
            id: "ignore-list-test".to_string(),
            name: "ignore-list-test".to_string(),
            game_dir: std::env::temp_dir()
                .join("love-launcher-ignorelist-test")
                .to_string_lossy()
                .to_string(),
            version: version.clone(),
        };
        let account = Account {
            uuid: "0".repeat(32),
            name: "OfflineTester".to_string(),
            access_token: Some("fake-token".to_string()),
            refresh_token: None,
            expires_at: None,
        };
        let cfg = LaunchConfig {
            java_path: java,
            memory_mb: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
        };
        let (_cmd, summary) = build_command(&instance, &meta, &account, &cfg).expect("组装命令失败");
        let list = summary
            .split(' ')
            .find(|a| a.starts_with("-DignoreList="))
            .expect("命令行里没有 -DignoreList");
        println!("{list}");
        if let Some(parent) = meta.inherits_from.clone() {
            assert!(
                list.contains(&format!("{parent}.jar")),
                "ignoreList 少了继承的原版 jar：{list}"
            );
        }
        assert!(
            list.contains(&format!("{}.jar", meta.id)),
            "ignoreList 少了加载器 jar：{list}"
        );
    }

    /// 半程验证：用离线 UUID + 假令牌拉起 Java，确认 classpath/参数组装与进程拉起正确。
    /// cargo test --lib live_launch -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_launch_offline_account() {
        let version = "1.20.1";
        if !download::is_installed(version) {
            println!("skip: {version} 还没安装");
            return;
        }
        let meta = download::load_merged_meta(version).expect("读取版本元数据失败");
        let java = std::env::var("TEST_JAVA")
            .unwrap_or_else(|_| r"C:\Dev\JDK\jdk-21.0.13+9\bin\java.exe".to_string());
        let game_dir = std::env::temp_dir().join("love-launcher-launch-test");
        std::fs::create_dir_all(&game_dir).unwrap();

        let instance = Instance {
            id: "live-test".to_string(),
            name: "live-test".to_string(),
            game_dir: game_dir.to_string_lossy().to_string(),
            version: version.to_string(),
        };
        let account = Account {
            uuid: "00000000000000000000000000000000".to_string(),
            name: "OfflineTester".to_string(),
            access_token: Some("fake-token-for-arg-check".to_string()),
            refresh_token: None,
            expires_at: None,
        };
        let cfg = LaunchConfig {
            java_path: java,
            memory_mb: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
        };

        let outcome = launch(&instance, &meta, &account, &cfg, None).expect("拉起 Java 失败");
        println!("pid={} log={}", outcome.pid, outcome.log_path);
        println!("summary={}", outcome.summary);
        std::thread::sleep(std::time::Duration::from_secs(15));
        let log = std::fs::read_to_string(&outcome.log_path).unwrap_or_default();
        let head: String = log.chars().take(2500).collect();
        println!("---- log head ----\n{head}");
    }
}
