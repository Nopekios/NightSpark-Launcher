<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import SideNav from "./components/SideNav.vue";
import HomeView from "./views/HomeView.vue";
import AccountView from "./views/AccountView.vue";
import DownloadView from "./views/DownloadView.vue";
import SettingsView from "./views/SettingsView.vue";
import SmoothCursor from "./components/ui/smooth-cursor/SmoothCursor.vue";
import SleekLineCursor from "./components/ui/sleek-line-cursor/SleekLineCursor.vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  api,
  errText,
  STAGE_TEXT,
  type Account,
  type GameInstance,
  type InstallProgress,
} from "./api";

type ViewKey = "home" | "download" | "account" | "settings";

const active = ref<ViewKey>("home");
const instances = ref<GameInstance[]>([]);
const account = ref<Account | null>(null);
const busy = ref(false);
const launchingId = ref<string | null>(null);
const runningPids = ref<Record<string, number>>({});
const logPanel = ref<{ title: string; text: string; instanceId: string } | null>(null);
// 日志面板打开时跟着游戏跑，用户不用手动点刷
let logTimer = 0;
const toast = ref<{ text: string; tone: "ok" | "err" } | null>(null);
let toastTimer = 0;
// 全局任务条：不管是装 Java、导入整合包、补齐 mods 还是装加载器，
// 只要后端 emit 了 install-progress / loader-progress，这里都能看见。
const task = ref<{ stage: string; done: number; total: number } | null>(null);
let taskTimer = 0;
let unlistenExit: (() => void) | null = null;
let unlistenDrop: (() => void) | null = null;
/** 文件拖到窗口上时亮遮罩，不然用户以为拖不进来 */
const dropActive = ref(false);
let unlistenProgress: (() => void) | null = null;
let unlistenLoader: (() => void) | null = null;

function notify(text: string, tone: "ok" | "err" = "ok") {
  toast.value = { text, tone };
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toast.value = null;
  }, 3200);
}

const taskPercent = computed(() => {
  const t = task.value;
  if (!t || t.total <= 0) return null;
  return Math.min(100, Math.round((t.done / t.total) * 100));
});

const taskStage = computed(() => {
  const t = task.value;
  if (!t) return "";
  const label = STAGE_TEXT[t.stage] ?? t.stage;
  return t.total > 0 ? `${label} ${t.done}/${t.total}` : label;
});

function showTask(payload: InstallProgress) {
  window.clearTimeout(taskTimer);
  // 0/0 这种事件算不出百分比、也判定不了完成，直接不上屏，
  // 否则任务条会一直停在那儿转，看着像死锁。
  if (!payload.total || payload.total <= 0) {
    task.value = null;
    return;
  }
  task.value = { stage: payload.stage, done: payload.done, total: payload.total };
  // 阶段走完先停一会儿再收，免得多个阶段之间闪。
  if (payload.done >= payload.total) {
    taskTimer = window.setTimeout(() => {
      task.value = null;
    }, 1500);
  } else {
    // 兜底：长时间没有新事件就收掉，网络卡住时任务条也不会永远挂着。
    taskTimer = window.setTimeout(() => {
      task.value = null;
    }, 5 * 60 * 1000);
  }
}

async function loadInstances() {
  try {
    instances.value = await api.listInstances();
  } catch (err) {
    notify(errText(err), "err");
  }
}

async function loadAccount() {
  try {
    account.value = await api.getAccount();
  } catch (err) {
    notify(errText(err), "err");
  }
}

async function createInstance(payload: { name: string; version: string }) {
  if (busy.value) return;
  busy.value = true;
  try {
    await api.createInstance(payload.name, payload.version);
    notify(`实例「${payload.name}」已创建`);
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function importLocalVersions() {
  if (busy.value) return;
  busy.value = true;
  try {
    const added = await api.importLocalVersions();
    if (added.length === 0) {
      notify("没有发现可导入的版本", "err");
      return;
    }
    // 只登记版本不搬文件的话启动出来是纯净原版，这里把搬了多少 mod 说清楚
    const files = added.reduce((sum, item) => sum + item.files, 0);
    const mods = added.reduce((sum, item) => sum + item.mods, 0);
    const filled = added.filter((item) => item.reused).length;
    notify(
      `处理了 ${added.length} 个版本，搬入 ${files} 个文件（mods ${mods} 个）${
        filled > 0 ? `，其中 ${filled} 个是补齐已有实例` : ""
      }`
    );
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function importModpack(path: string) {
  if (busy.value) return;
  const label = path.split(/[\\/]/).pop() ?? path;
  busy.value = true;
  notify(`正在导入 ${label}…`);
  try {
    const report = await api.importModpack(path);
    const warn = report.warnings[0];
    notify(
      `已导入「${report.instance.name}」（${report.kind} 格式 · ${report.files} 个文件）${
        warn ? `，${warn}` : ""
      }`
    );
    active.value = "home";
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function syncInstanceFiles(item: GameInstance) {
  if (busy.value) return;
  busy.value = true;
  notify(`正在把「${item.name}」版本目录里的文件补进实例…`);
  try {
    const report = await api.syncInstanceFiles(item.id);
    const warn = report.warnings[0];
    notify(
      `已补 ${report.files} 个文件（其中 mods ${report.mods} 个）${warn ? `，${warn}` : ""}`
    );
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function fetchPendingMods(item: GameInstance) {
  if (busy.value) return;
  busy.value = true;
  notify(`正在补齐「${item.name}」的 mods…`);
  try {
    const report = await api.downloadPendingMods(item.id);
    const summary = `下载 ${report.downloaded} 个 · 已有 ${report.skipped} 个 · 未完成 ${report.failed} 个`;
    const detail = report.messages[0];
    notify(report.failed > 0 ? `${summary}，${detail ?? "看看清单"}` : summary, report.failed > 0 ? "err" : "ok");
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function verifyInstance(item: GameInstance) {
  if (busy.value) return;
  busy.value = true;
  notify(`正在校验「${item.name}」…`);
  try {
    const report = await api.verifyInstance(item.id);
    const parts = [`核对 ${report.checked} 个 · 正常 ${report.ok} 个`];
    if (report.missing.length > 0) parts.push(`缺 ${report.missing.length} 个`);
    if (report.broken.length > 0) parts.push(`损坏 ${report.broken.length} 个`);
    if (report.unverified.length > 0) parts.push(`没写校验值 ${report.unverified.length} 个`);
    const first = report.missing[0] ?? report.broken[0];
    const bad = report.missing.length + report.broken.length > 0;
    notify(first ? `${parts.join(" · ")}，比如 ${first}` : parts.join(" · "), bad ? "err" : "ok");
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function installLoader(payload: { item: GameInstance; spec: string }) {
  if (busy.value) return;
  busy.value = true;
  notify(`正在给「${payload.item.name}」装 ${payload.spec}…`);
  try {
    const report = await api.installInstanceLoader(payload.item.id, payload.spec, null, null, null);
    const head = `${report.loader} ${report.loader_version} 装好了 · 新下 ${report.downloaded} 个 · 复用 ${report.skipped} 个`;
    const detail = report.messages[0];
    notify(detail ? `${head}，${detail}` : head, "ok");
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function removeInstance(item: GameInstance) {
  if (busy.value) return;
  busy.value = true;
  try {
    await api.deleteInstance(item.id);
    notify(`实例「${item.name}」已删除`);
    await loadInstances();
  } catch (err) {
    notify(errText(err), "err");
  } finally {
    busy.value = false;
  }
}

async function launchInstance(item: GameInstance) {
  if (launchingId.value) return;
  launchingId.value = item.id;
  try {
    const outcome = await api.launchInstance(item.id);
    runningPids.value = { ...runningPids.value, [item.id]: outcome.pid };
    const bits = [
      `${item.name} 已启动（PID ${outcome.pid}）`,
      outcome.java_note ?? "",
      ...(outcome.prepare_notes ?? []),
    ].filter((part) => part.length > 0);
    notify(bits.join(" · "));
    // 启动完直接把日志摊开，跑起来的内容自己往上滚
    void showLog(item, true);
  } catch (err) {
    notify(errText(err), "err");
    // 起不来的时候把日志甩出来，省得用户自己找
    void showLog(item);
  } finally {
    launchingId.value = null;
  }
}

function stopLogFollow() {
  if (logTimer) {
    window.clearInterval(logTimer);
    logTimer = 0;
  }
}

function closeLog() {
  stopLogFollow();
  logPanel.value = null;
}

/** follow=true 时每 1.5 秒重读一次尾部，游戏在跑就跟着刷。 */
async function showLog(item: GameInstance, follow = false) {
  stopLogFollow();
  try {
    const text = await api.readLatestLog(item.id, 300);
    logPanel.value = { title: `${item.name} · latest.log`, text, instanceId: item.id };
  } catch (err) {
    notify(errText(err), "err");
    return;
  }
  if (!follow) return;
  logTimer = window.setInterval(async () => {
    if (!logPanel.value || logPanel.value.instanceId !== item.id) {
      stopLogFollow();
      return;
    }
    try {
      const next = await api.readLatestLog(item.id, 300);
      if (logPanel.value && logPanel.value.instanceId === item.id) {
        logPanel.value = { ...logPanel.value, text: next };
      }
    } catch {
      stopLogFollow();
    }
  }, 1500);
}

async function killGame(item: GameInstance) {
  const pid = runningPids.value[item.id];
  if (!pid) return;
  try {
    await api.killGameProcess(pid);
    notify(`已结束「${item.name}」（PID ${pid}）`);
  } catch (err) {
    notify(errText(err), "err");
  }
}

function onAccountChanged(next: Account | null) {
  account.value = next;
}

// 兜底：任何时候切回「我的实例」，都拉一次最新列表，避免外部新建的实例看不到。
watch(active, (next) => {
  if (next === "home") void loadInstances();
});

onMounted(async () => {
  await Promise.all([loadInstances(), loadAccount()]);
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "enter" || payload.type === "over") {
        dropActive.value = true;
        return;
      }
      if (payload.type === "leave") {
        dropActive.value = false;
        return;
      }
      dropActive.value = false;
      const zip = payload.paths.find((item) => item.toLowerCase().endsWith(".zip"));
      // 不是 zip 就当目录处理：PCL 版本隔离的 mods 全在 versions/<版本>/ 里
      const dir = payload.paths.find((item) => !/\.[a-z0-9]{1,6}$/i.test(item));
      const target = zip ?? dir ?? payload.paths[0];
      if (!target) {
        notify("没拿到拖进来的路径，改用「导入本机已装版本」", "err");
        return;
      }
      void importModpack(target);
    });
  } catch (err) {
    notify(`拖拽通道没起来：${errText(err)}，先用「导入本机已装版本」`, "err");
  }
  unlistenProgress = await api.onInstallProgress(showTask);
  unlistenLoader = await api.onLoaderProgress(showTask);
  unlistenExit = await api.onGameExit((pid) => {
    const hit = Object.entries(runningPids.value).find(([, value]) => value === pid);
    runningPids.value = Object.fromEntries(
      Object.entries(runningPids.value).filter(([, value]) => value !== pid)
    );
    const item = hit ? instances.value.find((it) => it.id === hit[0]) : undefined;
    if (!item) {
      notify(`游戏进程已退出（PID ${pid}）`);
      return;
    }
    notify(`「${item.name}」已退出，日志在下面`);
    // 退出时直接把这一局的日志摊开：崩没崩一眼就有答案
    void showLog(item);
  });
});

onUnmounted(() => {
  unlistenExit?.();
  unlistenDrop?.();
  unlistenProgress?.();
  unlistenLoader?.();
  stopLogFollow();
  window.clearTimeout(taskTimer);
});
</script>

<template>
  <div class="app-shell">
    <SleekLineCursor :trails="22" :size="46" :friction="0.52" />
    <SmoothCursor />
    <SideNav :active="active" :account="account" @navigate="active = $event" />
    <div v-if="dropActive" class="drop-veil">
      <div class="drop-card">
        <strong>松手就导入</strong>
        <span>整合包 zip，或者 versions 下某个版本文件夹，都能认</span>
      </div>
    </div>


    <Transition name="view" mode="out-in">
    <HomeView
      v-if="active === 'home'"
      :instances="instances"
      :busy="busy"
      :has-account="!!account"
      :launching-id="launchingId"
      :running-pids="runningPids"
      @create="createInstance"
      @remove="removeInstance"
      @open-account="active = 'account'"
      @launch="launchInstance"
      @show-log="showLog" @kill="killGame"
    @import-local="importLocalVersions"
        @fetch-mods="fetchPendingMods"
      @sync-files="syncInstanceFiles"
        @install-loader="installLoader"
        @verify="verifyInstance"
    />
    <DownloadView
      v-else-if="active === 'download'"
      @notify="notify"
      @instances-changed="loadInstances"
    />
    <AccountView
      v-else-if="active === 'account'"
      :account="account"
      @changed="onAccountChanged"
      @notify="notify"
    />
    <SettingsView v-else @notify="notify" />
    </Transition>

    <Transition name="toast">
      <div v-if="logPanel" class="modal-mask" @click.self="closeLog">
        <div class="log-modal">
          <header>
            <span class="log-dot" />
            <strong>{{ logPanel.title }}</strong>
            <button class="log-close" @click="closeLog">关闭</button>
          </header>
          <pre>{{ logPanel.text }}</pre>
        </div>
      </div>
    </Transition>

    <Transition name="toast">
      <div v-if="task" class="task-bar">
        <div class="task-info">
          <span class="task-stage">{{ taskStage }}</span>
          <span v-if="taskPercent !== null" class="task-percent">{{ taskPercent }}%</span>
        </div>
        <div class="task-track">
          <div
            class="task-fill"
            :class="{ moving: taskPercent === null }"
            :style="taskPercent === null ? {} : { width: `${taskPercent}%` }"
          />
        </div>
      </div>
    </Transition>

    <Transition name="toast">
      <div v-if="toast" class="toast" :class="toast.tone">{{ toast.text }}</div>
    </Transition>
  </div>
</template>

<style scoped>
.app-shell {
  position: relative;
  display: flex;
  height: 100%;
  min-height: 0;
  isolation: isolate;
}

/* ── 全局任务条：玻璃胶囊 + 流光进度 ── */
.task-bar {
  position: fixed;
  left: 50%;
  bottom: 26px;
  z-index: 40;
  width: min(480px, 76vw);
  transform: translateX(-50%);
  padding: 13px 18px 15px;
  border-radius: 16px;
  border: 1px solid rgba(0, 149, 255, 0.24);
  background: linear-gradient(180deg, rgba(24, 38, 52, 0.92), rgba(12, 18, 27, 0.95));
  -webkit-backdrop-filter: blur(22px) saturate(150%);
  backdrop-filter: blur(22px) saturate(150%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 26px 60px -24px rgba(0, 0, 0, 0.98),
    0 0 30px -12px rgba(0, 149, 255, 0.55);
  font-size: 12.5px;
}

.task-info {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 10px;
  color: var(--text-dim);
}

.task-stage {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  letter-spacing: 0.01em;
}

.task-percent {
  flex: none;
  font-weight: 700;
  letter-spacing: -0.01em;
  font-variant-numeric: tabular-nums;
  color: #ffffff;
}

.task-track {
  position: relative;
  height: 6px;
  border-radius: 999px;
  background: rgba(126, 160, 199, 0.14);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.6);
  overflow: hidden;
}

.task-fill {
  position: relative;
  height: 100%;
  border-radius: 999px;
  background: linear-gradient(90deg, #0072c6, #3eb1ff 58%, #35e0c8);
  box-shadow: 0 0 14px rgba(0, 149, 255, 0.75);
  transition: width 0.4s cubic-bezier(0.22, 1, 0.36, 1);
  overflow: hidden;
}

.task-fill::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.5), transparent);
  animation: ll-shine 1.9s ease-in-out infinite;
}

.task-fill.moving {
  width: 34%;
  animation: task-slide 1.15s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}

@keyframes task-slide {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(320%);
  }
}

/* ── 日志弹层 ── */
.modal-mask {
  position: fixed;
  inset: 0;
  z-index: 70;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  background: rgba(4, 8, 13, 0.72);
  -webkit-backdrop-filter: blur(10px) saturate(120%);
  backdrop-filter: blur(10px) saturate(120%);
}

.log-modal {
  display: flex;
  flex-direction: column;
  width: min(880px, 90vw);
  max-height: 76vh;
  border-radius: 20px;
  border: 1px solid rgba(126, 160, 199, 0.18);
  background: linear-gradient(180deg, #0f1826 0%, #0a111b 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 40px 90px -30px rgba(0, 0, 0, 1);
  overflow: hidden;
}

.log-modal header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 16px 14px 18px;
  border-bottom: 1px solid rgba(126, 160, 199, 0.12);
  font-size: 13px;
}

.log-modal header strong {
  flex: 1;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-weight: 600;
  letter-spacing: 0.005em;
}

.log-dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: #2fd07f;
  box-shadow: 0 0 10px rgba(47, 208, 127, 0.85);
}

.log-close {
  padding: 6px 14px;
  font-size: 12px;
  border-radius: 999px;
}

.log-modal pre {
  flex: 1;
  margin: 0;
  padding: 16px 20px 20px;
  overflow: auto;
  background: rgba(0, 0, 0, 0.24);
  font-family: "Cascadia Code", "JetBrains Mono", Consolas, monospace;
  font-size: 11.5px;
  line-height: 1.75;
  color: #9fb3c8;
  white-space: pre-wrap;
  word-break: break-all;
}

/* ── 拖拽导入遮罩 ── */
.drop-veil {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(4, 8, 13, 0.66);
  -webkit-backdrop-filter: blur(8px);
  backdrop-filter: blur(8px);
}

.drop-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 28px 40px;
  border-radius: 20px;
  border: 1px dashed rgba(0, 149, 255, 0.55);
  background: rgba(14, 24, 38, 0.9);
  text-align: center;
  box-shadow: 0 0 0 1px rgba(0, 149, 255, 0.12), 0 30px 70px -30px rgba(0, 0, 0, 0.95),
    0 0 60px -20px rgba(0, 149, 255, 0.5);
  animation: ll-float 4.2s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}

.drop-card strong {
  font-size: 15px;
  letter-spacing: -0.01em;
}

.drop-card span {
  font-size: 12.5px;
  color: var(--text-dim);
}
</style>
