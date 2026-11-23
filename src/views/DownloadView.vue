<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { api, errText, STAGE_TEXT, type InstallProgress, type VersionEntry } from "../api";
import MarketPanel from "../components/MarketPanel.vue";
import MorphingTabs from "@/components/ui/morphing-tabs/MorphingTabs.vue";

const emit = defineEmits<{
  (e: "notify", text: string, tone?: "ok" | "err"): void;
  /** 市场里导入整合包/装完资源后，实例集合变了，往上传给 App 刷新「我的实例」。 */
  (e: "instances-changed"): void;
}>();

// 顶层分两个区：官方版本照旧，资源市场走 CurseForge / Modrinth。
const tab = ref<"vanilla" | "market">("vanilla");

// inspira MorphingTabs 用的是中文标签，这里做双向映射
const TAB_LABELS = ["官方版本", "资源市场"] as const;
const activeLabel = computed(() => (tab.value === "vanilla" ? TAB_LABELS[0] : TAB_LABELS[1]));
function switchTab(label: string) {
  tab.value = label === TAB_LABELS[1] ? "market" : "vanilla";
}

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
    <MorphingTabs
      class="tab-goo"
      :tabs="[...TAB_LABELS]"
      :active-tab="activeLabel"
      :margin="16"
      :blur-std-dev="5"
      @update:activeTab="switchTab"
    />

    <Transition name="pane" mode="out-in">

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
    </Transition>
  </section>
</template>

<style scoped>
/* ── 分段控件 ── */
.tabs {
  display: inline-flex;
  gap: 4px;
  width: fit-content;
  margin-bottom: 20px;
  padding: 4px;
  border-radius: 12px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: rgba(6, 11, 18, 0.7);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.5);
}

.tabs button {
  padding: 7px 18px;
  border: none;
  border-radius: 9px;
  background: transparent;
  box-shadow: none;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-dim);
  transition: color 0.2s ease, background 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.22s ease;
}

.tabs button:hover:not(.active) {
  color: var(--text);
  background: rgba(126, 160, 199, 0.08);
  transform: none;
  box-shadow: none;
}

.tabs button.active {
  color: #eaf6ff;
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.3) 0%, rgba(0, 149, 255, 0.12) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18), 0 8px 20px -12px rgba(0, 149, 255, 0.95);
}

/* inspira MorphingTabs：goo 融合分页，这里把默认亮蓝块压成暗玻璃配色 */
.tab-goo {
  width: fit-content;
  margin-bottom: 20px;
  border-radius: 12px;
}

.tab-goo :deep(button) {
  border-radius: 9px;
  background: rgba(20, 32, 47, 0.92);
  color: var(--text-dim);
  font-size: 12.5px;
  font-weight: 600;
  padding: 7px 18px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06);
}

.tab-goo :deep(button:hover) {
  color: var(--text);
}

/* 分页：内容区左右滑入 */
.pane-enter-active {
  transition: opacity 0.26s ease, transform 0.34s cubic-bezier(0.22, 1, 0.36, 1);
}

.pane-leave-active {
  transition: opacity 0.16s ease, transform 0.16s ease;
}

.pane-enter-from {
  opacity: 0;
  transform: translateX(22px);
}

.pane-leave-to {
  opacity: 0;
  transform: translateX(-16px);
}

.head-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 12px;
}

/* 复选开关 */
.switch {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-radius: 999px;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: rgba(255, 255, 255, 0.03);
  font-size: 12px;
  color: var(--text-dim);
  cursor: pointer;
  transition: border-color 0.2s ease, color 0.2s ease, background 0.2s ease;
}

.switch:hover {
  color: var(--text);
  border-color: rgba(0, 149, 255, 0.3);
  background: rgba(0, 149, 255, 0.06);
}

.switch input {
  appearance: none;
  -webkit-appearance: none;
  position: relative;
  width: 28px;
  height: 16px;
  padding: 0;
  border-radius: 999px;
  border: 1px solid rgba(126, 160, 199, 0.28);
  background: rgba(126, 160, 199, 0.16);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.5);
  cursor: pointer;
  transition: background 0.22s ease, border-color 0.22s ease;
}

.switch input::after {
  content: "";
  position: absolute;
  top: 1px;
  left: 1px;
  width: 12px;
  height: 12px;
  border-radius: 999px;
  background: #cddced;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.6);
  transition: transform 0.22s cubic-bezier(0.22, 1, 0.36, 1), background 0.22s ease;
}

.switch input:checked {
  border-color: rgba(0, 149, 255, 0.6);
  background: linear-gradient(90deg, #0072c6, #3eb1ff);
}

.switch input:checked::after {
  transform: translateX(12px);
  background: #ffffff;
}

/* ── 安装进度 ── */
.progress-panel {
  margin-bottom: 18px;
  padding: 15px 18px;
  border-radius: 14px;
  border: 1px solid rgba(0, 149, 255, 0.24);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.09) 0%, rgba(0, 149, 255, 0.02) 100%),
    rgba(10, 16, 24, 0.9);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 16px 40px -24px rgba(0, 149, 255, 0.85);
}

.progress-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 14px;
  margin-bottom: 9px;
  font-size: 12px;
  color: var(--text-dim);
}

.bar {
  position: relative;
  height: 6px;
  border-radius: 999px;
  background: rgba(126, 160, 199, 0.14);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.6);
  overflow: hidden;
}

.bar-fill {
  position: relative;
  height: 100%;
  border-radius: 999px;
  background: linear-gradient(90deg, #0072c6, #3eb1ff 58%, #35e0c8);
  box-shadow: 0 0 14px rgba(0, 149, 255, 0.7);
  transition: width 0.4s cubic-bezier(0.22, 1, 0.36, 1);
  overflow: hidden;
}

.bar-fill::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.45), transparent);
  animation: ll-shine 1.9s ease-in-out infinite;
}

.hint {
  margin: 8px 0 0;
  font-size: 11.5px;
  color: var(--text-faint);
}

/* ── 搜索栏 ── */
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  margin-bottom: 14px;
}

.search {
  width: 260px;
  padding: 8px 15px;
  border-radius: 999px;
  background: rgba(10, 18, 28, 0.9);
}

.count {
  font-size: 11.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-faint);
  font-variant-numeric: tabular-nums;
}

/* ── 版本列表 ── */
.list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 15px 18px;
  border-radius: 13px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.035) 0%, rgba(255, 255, 255, 0.008) 100%),
    rgba(12, 18, 27, 0.82);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  transition: border-color 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.22s cubic-bezier(0.22, 1, 0.36, 1), transform 0.22s cubic-bezier(0.22, 1, 0.36, 1);
}

.row:hover {
  border-color: rgba(0, 149, 255, 0.32);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 18px 38px -24px rgba(0, 149, 255, 0.85);
  transform: translateY(-1px);
}

.row-main {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.row-main h2 {
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 0;
  font-size: 14px;
  font-weight: 650;
  letter-spacing: -0.01em;
  font-variant-numeric: tabular-nums;
}

.row-main p {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-faint);
  letter-spacing: 0.01em;
}

.row-side {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: none;
}

.state {
  font-size: 11.5px;
  font-weight: 650;
  color: var(--text-faint);
}

.state.ok {
  color: #2fd07f;
}

.notice.err {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  margin-bottom: 16px;
  padding: 13px 16px;
  border-radius: 12px;
  border: 1px solid rgba(255, 90, 110, 0.34);
  background: rgba(255, 90, 110, 0.07);
  font-size: 12px;
  color: #ffc2ca;
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skeleton {
  height: 58px;
}

.more {
  align-self: center;
  margin-top: 6px;
  padding: 9px 22px;
  border-radius: 999px;
}

.foot {
  margin: 18px 0 0;
  font-size: 11.5px;
  line-height: 1.75;
  color: var(--text-faint);
}

.foot code {
  padding: 2px 6px;
  border-radius: 5px;
  background: rgba(126, 160, 199, 0.1);
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
  color: #9fb3c8;
}

.subpage {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
/* 版本类型标签修饰 */
.tag {
  flex: none;
  padding: 2px 9px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  border: 1px solid rgba(126, 160, 199, 0.16);
  background: rgba(126, 160, 199, 0.08);
  color: var(--text-dim);
}

.tag.ok {
  color: #bfe0ff;
  background: rgba(0, 149, 255, 0.12);
  border-color: rgba(0, 149, 255, 0.24);
}

.tag.muted {
  color: #c6b6a0;
  background: rgba(245, 185, 66, 0.1);
  border-color: rgba(245, 185, 66, 0.24);
}
/* ── 版本卡阵：不再是一条条横排列表 ── */
.list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(246px, 1fr));
  gap: 12px;
}

.list .row {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  min-height: 132px;
  padding: 16px 18px 14px;
  border-radius: 16px;
  overflow: hidden;
  transition: transform 0.3s cubic-bezier(0.22, 1, 0.36, 1), border-color 0.3s ease,
    box-shadow 0.3s ease;
}

.list .row::after {
  content: "";
  position: absolute;
  left: 18%;
  right: 18%;
  top: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(0, 149, 255, 0.9), transparent);
  opacity: 0;
  transition: opacity 0.3s ease;
}

.list .row:hover {
  transform: translateY(-4px);
  border-color: rgba(0, 149, 255, 0.32);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 34px 60px -42px rgba(0, 149, 255, 0.75);
}

.list .row:hover::after {
  opacity: 1;
}

.list .row h2 {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin: 0;
  font-size: 17px;
  font-weight: 700;
  letter-spacing: -0.02em;
  font-variant-numeric: tabular-nums;
}

.list .row p {
  margin: 0;
  font-size: 11px;
  color: var(--text-faint);
}

.list .row-side {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-top: auto;
  padding-top: 11px;
  border-top: 1px solid rgba(126, 160, 199, 0.1);
}

/* 骨架也铺成网格，避免加载时突然从单列跳成网格 */
.skeleton-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(246px, 1fr));
  gap: 12px;
}

.skeleton-list .skeleton {
  min-height: 132px;
}

/* 安装进度条：流光 */
.bar-fill {
  background: linear-gradient(90deg, #2f9bff, #35e0c8, #2f9bff);
  background-size: 220% 100%;
  animation: ll-btn-flow 2.6s linear infinite;
  box-shadow: 0 0 18px rgba(0, 149, 255, 0.75);
}

.list .row {
  transition: transform 0.3s cubic-bezier(0.22, 1, 0.36, 1), border-color 0.3s ease,
    box-shadow 0.3s ease;
}

.list .row:hover h2 {
  color: #dff0ff;
  text-shadow: 0 0 18px rgba(0, 149, 255, 0.55);
}

</style>
