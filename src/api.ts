import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type GameInstance = {
  id: string;
  name: string;
  version: string;
  game_dir: string;
};

export type Watermark = {
  tag: string;
  serial: string;
  commit: string;
  built_at: string;
  version: string;
  official: boolean;
  integrity: boolean;
};

export type JavaInfo = {
  path: string;
  version: string;
};

/** 账号类型：msa = 正版微软账号，offline = 纯本地离线账号。 */
export type AccountKind = "msa" | "offline";

export type Account = {
  uuid: string;
  name: string;
  expiresAt: number | null;
  hasRefreshToken: boolean;
  /** 是否是当前正在用的账号（启动游戏取它的令牌）。 */
  active: boolean;
  kind: AccountKind;
};

export type VersionEntry = {
  id: string;
  type: string;
  url: string;
  releaseTime: string;
};

export type InstallReport = {
  version: string;
  java_major: number;
  libraries: number;
  assets: number;
};

export type InstallProgress = {
  stage: string;
  done: number;
  total: number;
};

export type LaunchOutcome = {
  pid: number;
  log_path: string;
  summary: string;
  /** 自动换了 Java 时的说明，没换就没有这个字段。 */
  java_note?: string | null;
  /** 启动前自动补的东西（搬整合包文件、补 mod、下版本文件），按顺序展示。 */
  prepare_notes?: string[];
};

/** 启动器自己下载安装的 Java 运行时。 */
export type JavaRuntime = {
  major: number;
  path: string;
  version: string;
};

export type ImportReport = {
  instance: GameInstance;
  kind: string;
  files: number;
  /** 整合包要求的加载器写法，如 forge-47.2.0；纯原版包为 null */
  loader: string | null;
  warnings: string[];
};

export type PendingReport = {
  downloaded: number;
  skipped: number;
  failed: number;
  messages: string[];
};

/** 加载器装完之后回给前端的结果。 */
export type LoaderReport = {
  loader: string;
  loader_version: string;
  mc_version: string;
  /** 装完要用的版本 id，形如 1.20.1-forge-47.2.0 */
  target_version: string;
  version_json: string;
  downloaded: number;
  skipped: number;
  messages: string[];
};

/** 实例文件校验的结果。 */
export type VerifyReport = {
  checked: number;
  ok: number;
  missing: string[];
  broken: string[];
  unverified: string[];
  /** 实例 mods 目录下的 jar 数 */
  mods_jars: number;
  notes: string[];
};

/** 导入本机版本的结果：搬了哪些文件、补了多少 mod。 */
export type ImportedInstance = {
  instance: GameInstance;
  files: number;
  mods: number;
  /** 实例本来就在，这次只是把版本隔离的内容补进去 */
  reused: boolean;
};

/** 版本隔离文件搬运统计。 */
export type CarryReport = {
  files: number;
  mods: number;
  linked: number;
  copied: number;
  kept: number;
  warnings: string[];
};

export type LauncherConfig = {
  java_path: string | null;
  memory_mb: number;
  game_dir: string | null;
  /** null 表示用户还没选过语言，首次启动要弹选择框 */
  language: string | null;
};

/** 资源市场里的一条项目（CurseForge 或 Modrinth）。 */
export type MarketProject = {
  source: string;
  id: string;
  slug: string;
  name: string;
  summary: string;
  icon: string | null;
  downloads: number;
  author: string;
  kind: string;
};

/** 项目下的一个可下载文件。 */
export type MarketFile = {
  name: string;
  version: string;
  gameVersions: string[];
  loaders: string[];
  url: string;
  size: number | null;
  sha1: string | null;
  folder: string;
};

/** 装完一个市场文件的结果。 */
export type MarketInstallReport = {
  fileName: string;
  folder: string;
  path: string;
  skipped: boolean;
};

export function errText(err: unknown): string {
  if (err && typeof err === "object") {
    const e = err as { code?: string; message?: string };
    const parts = [e.code, e.message].filter((v): v is string => typeof v === "string" && v.length > 0);
    if (parts.length > 0) return parts.join(" · ");
  }
  return String(err);
}

/** 安装进度事件的阶段名 → 当前语言的说明；没有对应键时原样返回阶段名。 */
export function stageText(stage: string): string {
  const key = `stage.${stage.replace(/-/g, "_")}`;
  const text = t(key);
  return text === key ? stage : text;
}

export const api = {
  getWatermark: () => invoke<Watermark>("watermark_info"),
  ping: () => invoke<string>("ping"),
  javaList: () => invoke<JavaInfo[]>("get_java_list"),
  installJava: (major: number) => invoke<JavaRuntime>("install_java", { major }),
  listJavaRuntimes: () => invoke<JavaRuntime[]>("list_java_runtimes"),
  listInstances: () => invoke<GameInstance[]>("list_instances"),
  createInstance: (name: string, version: string) =>
    invoke<GameInstance>("create_instance", { name, version }),
  deleteInstance: (id: string) => invoke<void>("delete_instance", { id }),
  importLocalVersions: () => invoke<ImportedInstance[]>("import_local_versions"),
  syncInstanceFiles: (instanceId: string) =>
    invoke<CarryReport>("sync_instance_files", { instanceId }),
  importModpack: (path: string) => invoke<ImportReport>("import_modpack", { path }),
  downloadPendingMods: (instanceId: string) =>
    invoke<PendingReport>("download_pending_mods", { instanceId }),
  verifyInstance: (instanceId: string) =>
    invoke<VerifyReport>("verify_instance", { instanceId }),
  installInstanceLoader: (
    instanceId: string,
    loader: string,
    loaderVersion?: string | null,
    mcVersion?: string | null,
    javaPath?: string | null,
  ) =>
    invoke<LoaderReport>("install_instance_loader", {
      instanceId,
      loader,
      loaderVersion: loaderVersion ?? null,
      mcVersion: mcVersion ?? null,
      javaPath: javaPath ?? null,
    }),
  startLogin: () => invoke<void>("start_login"),
  exchangeCode: (code: string) => invoke<Account>("exchange_code", { code }),
  getAccount: () => invoke<Account | null>("get_account"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  switchAccount: (uuid: string) => invoke<Account>("switch_account", { uuid }),
  removeAccount: (uuid: string) => invoke<void>("remove_account", { uuid }),
  refreshAccount: () => invoke<Account>("refresh_account"),
  /** 离线登录：只本地生成用户名 + UUID，不碰微软。 */
  loginOffline: (name: string) => invoke<Account>("login_offline", { name }),
  /** 取账号头像（Minecraft 皮肤头部）的 data URL。 */
  accountAvatar: (uuid: string, force = false) =>
    invoke<string>("account_avatar", { uuid, force }),
  logout: () => invoke<void>("logout"),

  /** 账号库变动（登录 / 切换 / 移除 / 后台自动续期）时推送完整列表。 */
  onAccountsChanged: (cb: (accounts: Account[]) => void): Promise<UnlistenFn> =>
    listen<Account[]>("accounts-changed", (event) => cb(event.payload)),

  getConfig: () => invoke<LauncherConfig>("get_config"),
  saveConfig: (cfg: LauncherConfig) => invoke<void>("save_config", { cfg }),

  fetchVersionList: () => invoke<VersionEntry[]>("fetch_version_list"),
  installVersion: (version: string, withAssets: boolean) =>
    invoke<InstallReport>("install_version", { version, withAssets }),
  isVersionInstalled: (version: string) => invoke<boolean>("is_version_installed", { version }),

  launchInstance: (id: string, javaPath?: string, memoryMb?: number) =>
    invoke<LaunchOutcome>("launch_instance", { id, javaPath, memoryMb }),
  readLatestLog: (id: string, maxLines = 200) =>
    invoke<string>("read_latest_log", { id, maxLines }),
  killGameProcess: (pid: number) => invoke<void>("kill_game_process", { pid }),

  searchMarket: (source: string, kind: string, query: string, page: number) =>
    invoke<MarketProject[]>("search_market", { source, kind, query, page }),
  listMarketFiles: (
    source: string,
    projectId: string,
    kind: string,
    gameVersion?: string | null,
    loader?: string | null,
  ) =>
    invoke<MarketFile[]>("list_market_files", {
      source,
      projectId,
      kind,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
    }),
  installMarketFile: (instanceId: string, file: MarketFile) =>
    invoke<MarketInstallReport>("install_market_file", { instanceId, file }),
  installMarketModpack: (file: MarketFile) =>
    invoke<ImportReport>("install_market_modpack", { file }),

  onInstallProgress: (cb: (p: InstallProgress) => void): Promise<UnlistenFn> =>
    listen<InstallProgress>("install-progress", (event) => cb(event.payload)),

  onLoaderProgress: (cb: (p: InstallProgress) => void): Promise<UnlistenFn> =>
    listen<InstallProgress>("loader-progress", (event) => cb(event.payload)),

  onGameExit: (cb: (pid: number) => void): Promise<UnlistenFn> =>
    listen<number>("game-exit", (event) => cb(event.payload)),
};
