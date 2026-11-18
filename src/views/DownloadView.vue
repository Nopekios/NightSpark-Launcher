<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { api, errText, STAGE_TEXT, type InstallProgress, type VersionEntry } from "../api";
import MarketPanel from "../components/MarketPanel.vue";

const emit = defineEmits<{
  (e: "notify", text: string, tone?: "ok" | "err"): void;
  /** 市场里导入整合包/装完资源后，实例集合变了，往上传给 App 刷新「我的实例」。 */
  (e: "instances-changed"): void;
}>();

// 顶层分两个区：官方版本照旧，资源市场走 CurseForge / Modrinth。
const tab = ref<"vanilla" | "market">("vanilla");

const versions = ref<VersionEntry[]>([]);
const loading = ref(false);
const loadErr = ref("");
const query = ref("");
const showSnapshots = ref(false);
const limit = ref(30);
const withAssets = ref(true);

const installedMap = ref<Record<string, boolean>>({});
const installing = ref<string | null>(null);
const progress = ref<InstallProgress | null>(null);

let unlisten: (() => void) | null = null;

const filtered = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  let list = versions.value;
  if (!showSnapshots.value) list = list.filter((v) => v.type === "release");
  if (keyword) list = list.filter((v) => v.id.toLowerCase().includes(keyword));
  return list;
});

const visible = computed(() => filtered.value.slice(0, limit.value));

const percent = computed(() => {
  const p = progress.value;
  if (!p || p.total === 0) return 0;
  return Math.min(100, Math.round((p.done / p.total) * 100));
});

const stageText = computed(() => {
  const p = progress.value;
  if (!p) return "";
  const label = STAGE_TEXT[p.stage] ?? p.stage;
  return p.total > 0 ? `${label} ${p.done}/${p.total}` : label;
});

async function loadVersions() {
  loading.value = true;
  loadErr.value = "";
  try {
    versions.value = await api.fetchVersionList();
  } catch (err) {
    loadErr.value = errText(err);
  } finally {
    loading.value = false;
  }
}

/** 只检查当前可见的版本，避免一次性打上百次 IPC。 */
async function checkInstalled() {
  for (const item of visible.value) {
    if (item.id in installedMap.value) continue;
    try {
      installedMap.value[item.id] = await api.isVersionInstalled(item.id);
    } catch {
      installedMap.value[item.id] = false;
    }
  }
}

async function install(item: VersionEntry) {
  if (installing.value) return;
  installing.value = item.id;
  progress.value = { stage: "manifest", done: 0, total: 0 };
  try {
    const report = await api.installVersion(item.id, withAssets.value);
    installedMap.value[item.id] = true;
    emit(
      "notify",
      `${report.version} 安装完成：${report.libraries} 个依赖库，${report.assets} 个资源文件`
    );
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    installing.value = null;
    progress.value = null;
  }
}

function dateText(raw: string) {
  return raw.slice(0, 10);
}

watch(visible, () => {
  void checkInstalled();
});

onMounted(async () => {
  await loadVersions();
  unlisten = await api.onInstallProgress((payload) => {
    progress.value = payload;
  });
});

onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <section class="page">
    <nav class="tabs">
      <button :class="{ active: tab === 'vanilla' }" @click="tab = 'vanilla'">官方版本</button>
      <button :class="{ active: tab === 'market' }" @click="tab = 'market'">资源市场</button>
    </nav>

    <div v-if="tab === 'vanilla'" class="subpage">
    <header class="page-head">
      <div>
        <h1>游戏版本</h1>
        <p>
          直接从 Mojang 官方清单拉取<span v-if="versions.length > 0">，当前共 {{ versions.length }} 个版本</span>
        </p>
      </div>
      <div class="head-actions">
        <label class="switch">
          <input v-model="showSnapshots" type="checkbox" />
          <span>显示快照</span>
        </label>
        <label class="switch">
          <input v-model="withAssets" type="checkbox" />
          <span>同时下载资源文件</span>
        </label>
        <button :disabled="loading || !!installing" @click="loadVersions">
          {{ loading ? "拉取中…" : "刷新清单" }}
        </button>
      </div>
    </header>

    <div v-if="installing" class="progress-panel panel">
      <div class="progress-head">
        <strong>正在安装 {{ installing }}</strong>
        <span>{{ stageText }}</span>
      </div>
      <div class="bar">
        <div class="bar-fill" :style="{ width: `${percent}%` }" />
      </div>
      <p class="hint">首次安装依赖库和资源会慢一些，重复安装会自动跳过已有文件。</p>
    </div>

    <div v-if="loadErr" class="notice err">
      <div>
        <strong>版本清单拉取失败</strong>
        <p>{{ loadErr }}</p>
      </div>
      <button @click="loadVersions">重试</button>
    </div>

    <div class="toolbar">
      <input v-model="query" class="search" placeholder="搜索版本号，例如 1.20.1" />
      <span class="count">{{ filtered.length }} 个匹配</span>
    </div>

    <div v-if="loading && versions.length === 0" class="skeleton-list">
      <div v-for="n in 6" :key="n" class="skeleton panel" />
    </div>

    <div v-else class="list">
      <article v-for="item in visible" :key="item.id" class="panel row">
        <div class="row-main">
          <h2>
            {{ item.id }}
            <span class="tag" :class="item.type === 'release' ? 'ok' : 'muted'">
              {{ item.type === "release" ? "正式版" : item.type === "snapshot" ? "快照" : item.type }}
            </span>
          </h2>
          <p>发布于 {{ dateText(item.releaseTime) }}</p>
        </div>
        <div class="row-side">
          <span v-if="installedMap[item.id]" class="state ok">已安装</span>
          <button
            class="primary"
            :disabled="!!installing"
            @click="install(item)"
          >
            {{ installing === item.id ? "安装中…" : installedMap[item.id] ? "重新校验" : "下载安装" }}
          </button>
        </div>
      </article>
    </div>

    <div v-if="filtered.length > visible.length" class="more">
      <button @click="limit += 30">再显示 30 个</button>
    </div>

    <p class="foot">
      安装位置：共享 <code>.minecraft</code> 目录下的 versions / libraries / assets，实例之间不会重复下载。
      资源文件较大（约 500MB），只想先跑起来可以关掉上面的开关。
    </p>
    </div>

    <MarketPanel
      v-else
      @notify="(text, tone) => emit('notify', text, tone)"
      @instances-changed="emit('instances-changed')"
    />
  </section>
</template>

<style scoped>
.tabs {
  display: inline-flex;
  gap: 6px;
  padding: 4px;
  margin-bottom: 18px;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: var(--radius-md);
  box-shadow: inset 0 -2px 0 rgba(0, 0, 0, 0.4);
}

.tabs button {
  padding: 6px 16px;
  font-size: 12.5px;
  font-weight: 700;
  letter-spacing: 0.3px;
  color: var(--text-dim);
  background: transparent;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}

.tabs button:hover {
  color: var(--text);
}

.tabs button.active {
  color: #e2ffe0;
  background: linear-gradient(180deg, rgba(98, 204, 85, 0.34), rgba(47, 122, 51, 0.34));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18);
}

.head-actions {
  display: flex;
  align-items: center;
  gap: 14px;
}

.switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-dim);
  cursor: pointer;
}

.progress-panel {
  margin-bottom: 18px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.progress-head {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-dim);
}

.bar {
  height: 6px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.08);
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  border-radius: 999px;
  background: linear-gradient(90deg, #7c5cff, #4ad7c8);
  transition: width 0.25s ease;
}

.hint {
  margin: 0;
  font-size: 11px;
  color: var(--text-faint);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 14px;
}

.search {
  width: 100%;
  max-width: 280px;
}

.count {
  font-size: 12px;
  color: var(--text-faint);
}

.list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 14px 18px;
}

.row-main h2 {
  margin: 0 0 4px;
  font-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.row-main p {
  margin: 0;
  font-size: 11px;
  color: var(--text-faint);
}

.row-side {
  display: flex;
  align-items: center;
  gap: 12px;
}

.state {
  font-size: 11px;
  color: var(--text-faint);
}

.state.ok {
  color: #4ad7c8;
}

.notice.err {
  border-color: rgba(255, 106, 106, 0.4);
  background: rgba(255, 106, 106, 0.08);
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.skeleton {
  height: 58px;
  opacity: 0.35;
  animation: pulse 1.4s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 0.2;
  }
  50% {
    opacity: 0.45;
  }
}

.more {
  display: flex;
  justify-content: center;
  margin-top: 16px;
}

.foot {
  margin: 20px 0 0;
  font-size: 11px;
  line-height: 1.8;
  color: var(--text-faint);
}

code {
  padding: 1px 5px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
  font-size: 11px;
}

/* ===== 下载页像素风强化 ===== */
.list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
}

.row {
  border: 2px solid #0b1015;
  border-radius: 5px;
  padding: 12px 16px;
}

.row:hover {
  border-color: rgba(98, 204, 85, 0.45);
}

.row-main h2 {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
  font-weight: 800;
  letter-spacing: 0.3px;
}

.row-main p {
  margin: 3px 0 0;
  font-size: 11.5px;
  color: var(--text-faint);
}

.tag.ok {
  color: #d6ffd2;
  background: linear-gradient(180deg, rgba(98, 204, 85, 0.34), rgba(47, 122, 51, 0.3));
  border: 1px solid rgba(98, 204, 85, 0.5);
}

.state.ok {
  color: var(--diamond);
  font-size: 11px;
  font-weight: 700;
}

.bar {
  height: 12px;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: 3px;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  background: linear-gradient(180deg, var(--grass-hi), var(--grass-deep));
  box-shadow: inset 0 2px 0 rgba(255, 255, 255, 0.25);
  transition: width 0.25s ease;
}

.foot {
  margin-top: 18px;
  font-size: 11.5px;
  line-height: 1.75;
  color: var(--text-faint);
}

.foot code {
  background: #0f151b;
  border: 1px solid #0a0e12;
  border-radius: 3px;
  padding: 1px 5px;
}

</style>
