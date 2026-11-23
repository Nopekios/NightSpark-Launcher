# NightSpark

NightSpark 是一个面向 Windows 桌面端的 Minecraft 启动器。上层用 Vue 3 + TypeScript 构建界面，下层由 Rust 承担版本下载、文件校验、账号认证、Java 运行时管理与游戏进程启动，两者通过 Tauri 2 的 IPC 命令通信。

- 应用标识：`com.toxic.nightspark`
- 当前版本：`0.1.0`

## 功能概览

| 模块 | 说明 |
| --- | --- |
| 实例管理 | 一个实例对应「游戏目录 + 版本 + 配置」；支持新建、删除、导入本地已有版本，实例之间共享已下载的版本与资源文件 |
| 版本下载 | 从官方版本清单拉取可用版本，按需下载 client jar、libraries、natives 与 assets，无需一次性拉全量资源 |
| 加载器安装 | 支持 Fabric / Quilt / Forge / NeoForge；Fabric、Quilt 走官方元数据 profile，Forge、NeoForge 调用官方 installer 完成 processors 处理 |
| 整合包导入 | 识别 PCL、HMCL 及通用压缩包结构，解压到实例独立目录并自动登记版本与加载器要求 |
| 微软账号登录 | 微软正版登录链路 MSA → Xbox Live → XSTS → Minecraft Services，支持令牌刷新与退出登录 |
| 启动游戏 | 按实例参数（Java 路径、内存上限）组装启动命令，可查询运行状态、读取最新日志、结束游戏进程 |
| 实例校验 | 检查实例文件完整性，区分缺失、损坏与未校验文件，并同步共享版本文件到实例 |
| 资源市场 | 接入 Modrinth 与 CurseForge，支持搜索项目、列出文件、安装到指定实例或按整合包安装；CurseForge 部分采用镜像优先、官方兜底的策略 |
| Java 运行时 | 从 Mojang 官方 java-runtime 清单按需安装，覆盖 Java 8 / 16 / 17 / 21 等主流大版本，避免用户手动配环境 |

## 技术栈

| 层次 | 选型 |
| --- | --- |
| 桌面框架 | Tauri 2（`tauri`、`tauri-plugin-opener`） |
| 后端 | Rust（edition 2024），tokio 异步运行时、reqwest 网络请求、serde / serde_json 序列化、zip 压缩包处理、dirs 目录解析、encoding_rs 编码转换 |
| 前端 | Vue 3（`<script setup>`）+ TypeScript + Vite |
| 样式与组件 | Tailwind CSS 4（`@tailwindcss/vite`）、reka-ui、class-variance-authority、clsx、tailwind-merge、motion-v、@vueuse/core、@lucide/vue |
| 打包 | `tauri build`，bundle 目标为 `all` |

## 目录结构

```
Love-Launcher/
├─ src/                         前端源码
│  ├─ api.ts                    后端命令的类型化封装（invoke 调用入口）
│  ├─ App.vue                   应用外壳与全局布局
│  ├─ views/                    页面：HomeView（实例）、DownloadView（版本下载）、AccountView（账号）、SettingsView（设置）
│  ├─ components/               业务组件：SideNav、MarketPanel 等
│  ├─ components/ui/            通用视觉组件（卡片、光标、背景、标签页等）
│  ├─ styles/global.css         全局样式与主题变量
│  └─ lib/utils.ts              工具函数
├─ src-tauri/                   后端源码
│  ├─ src/lib.rs                Tauri Builder 与命令注册
│  ├─ src/commands.rs           对前端暴露的命令层
│  ├─ src/auth/                 微软账号认证
│  ├─ src/config/               路径解析、配置读写、本地 Java 探测
│  ├─ src/download/             版本清单与资源下载
│  ├─ src/hash.rs               SHA1 等校验
│  ├─ src/instance/             实例管理、加载器安装、整合包导入、完整性校验
│  ├─ src/java/                 Java 运行时按需安装
│  ├─ src/launch/               启动参数组装与进程管理
│  ├─ src/market/               资源市场（Modrinth / CurseForge）
│  ├─ src/net/                  网络请求与镜像地址
│  ├─ capabilities/             Tauri 权限声明
│  └─ tauri.conf.json           应用配置
├─ index.html
├─ vite.config.ts
├─ tsconfig.json
└─ package.json
```

## 环境要求

- Windows 10 及以上
- Node.js（建议使用 LTS 版本）与 npm
- Rust stable 工具链（含 Cargo）
- Tauri 2 在 Windows 上的构建依赖（WebView2 运行时、MSVC 编译工具链）

## 开发与构建

```bash
# 安装前端依赖
npm install

# 前端开发服务器（Tauri 配置中 devUrl 为 http://localhost:1420）
npm run dev

# 桌面端开发调试（同时拉起前端与 Rust 后端）
npm run tauri dev

# 仅构建前端产物到 dist/
npm run build

# 打包桌面应用安装包
npm run tauri build
```

`src-tauri/Cargo.toml` 的 release 配置已开启 `lto`、`codegen-units = 1`、`opt-level = 3`、`strip` 与 `panic = "abort"`，用于压缩发布体积。

## 后端命令接口

前端统一通过 `src/api.ts` 调用 `invoke`，后端在 `src-tauri/src/lib.rs` 注册命令：

| 分组 | 命令 |
| --- | --- |
| 实例 | `list_instances`、`create_instance`、`delete_instance`、`import_local_versions`、`sync_instance_files`、`verify_instance` |
| 加载器与整合包 | `install_instance_loader`、`import_modpack`、`download_pending_mods` |
| 版本 | `fetch_version_list`、`install_version`、`is_version_installed` |
| 账号 | `start_login`、`exchange_code`、`get_account`、`refresh_account`、`logout` |
| 启动 | `launch_instance`、`kill_game_process`、`read_latest_log` |
| 资源市场 | `search_market`、`list_market_files`、`install_market_file`、`install_market_modpack` |
| Java | `get_java_list`、`install_java`、`list_java_runtimes` |
| 配置 | `get_config`、`save_config` |
| 其他 | `ping` |

## 数据与配置位置

- 游戏目录：优先读取环境变量 `MINECRAFT_DIR`，否则使用系统数据目录下的 `.minecraft`
- 启动器配置：`config_dir/NightSpark/config.json`，可用环境变量 `NIGHTSPARK_HOME` 覆盖配置根目录
- 可配置项：`java_path`（Java 可执行文件路径，留空则由启动器自动探测）、`memory_mb`（分配给游戏的最大内存，单位 MB）、`game_dir`（游戏目录覆盖值）
- 主窗口默认尺寸 1180×760，最小 960×640，支持拖拽文件

## 相关文档

- Tauri：https://v2.tauri.app
- Vue：https://vuejs.org
- Vite：https://vite.dev
- Tailwind CSS：https://tailwindcss.com
