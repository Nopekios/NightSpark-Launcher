<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  api,
  errText,
  type GameInstance,
  type InstallProgress,
  type MarketFile,
  type MarketProject,
} from "../api";

const emit = defineEmits<{
  (e: "notify", text: string, tone?: "ok" | "err"): void;
  /** 实例集合/实例内容发生变化，通知外层刷新「我的实例」，避免用户手动刷新。 */
  (e: "instances-changed"): void;
}>();

/** 资源类型：整合包下载后新建实例，其余装进选中实例。 */
const kinds = [
  { key: "mod", label: "模组" },
  { key: "modpack", label: "整合包" },
  { key: "resourcepack", label: "资源包" },
  { key: "shader", label: "光影" },
] as const;

/** 两条来源：CF 走镜像、Modrinth 走官方。 */
const sources = [
  { key: "curseforge", label: "CurseForge" },
  { key: "modrinth", label: "Modrinth" },
] as const;

const loaders = [
  { key: "", label: "不限" },
  { key: "forge", label: "Forge" },
  { key: "neoforge", label: "NeoForge" },
  { key: "fabric", label: "Fabric" },
  { key: "quilt", label: "Quilt" },
] as const;

const kind = ref<string>("mod");
const source = ref<string>("curseforge");
const query = ref("");
const gameVersion = ref("");
const loader = ref("");

const projects = ref<MarketProject[]>([]);
const page = ref(0);
const loading = ref(false);
const loadErr = ref("");
const hasMore = ref(false);

const expandedId = ref<string | null>(null);
const files = ref<MarketFile[]>([]);
const filesLoading = ref(false);
const filesErr = ref("");

const instances = ref<GameInstance[]>([]);
const targetInstance = ref("");
const busyName = ref<string | null>(null);
const progress = ref<InstallProgress | null>(null);

let unlisten: (() => void) | null = null;
let debounceTimer: number | undefined;

const PAGE_SIZE = 20;
const isModpack = computed(() => kind.value === "modpack");
const kindLabel = computed(() => kinds.find((item) => item.key === kind.value)?.label ?? "资源");
const percent = computed(() => {
  const current = progress.value;
  if (!current || !current.total) return 0;
  return Math.min(100, Math.round((current.done / current.total) * 100));
});

async function loadInstances() {
  try {
    instances.value = await api.listInstances();
    const known = instances.value.some((item) => item.id === targetInstance.value);
    if (!known && instances.value.length > 0) {
      targetInstance.value = instances.value[0].id;
    }
  } catch (err) {
    emit("notify", `读实例列表失败：${errText(err)}`, "err");
  }
}

async function search(reset = true) {
  if (reset) {
    page.value = 0;
    projects.value = [];
    expandedId.value = null;
    files.value = [];
  }
  loading.value = true;
  loadErr.value = "";
  try {
    const list = await api.searchMarket(source.value, kind.value, query.value, page.value);
    projects.value = reset ? list : [...projects.value, ...list];
    hasMore.value = list.length >= PAGE_SIZE;
    if (reset && list.length === 0) {
      loadErr.value = query.value.trim()
        ? "没搜到，换个关键词或者换条来源试试。"
        : "这条来源目前没返回内容，稍后再试或换条来源。";
    }
  } catch (err) {
    loadErr.value = errText(err);
    hasMore.value = false;
  } finally {
    loading.value = false;
  }
}

function loadMore() {
  page.value += 1;
  void search(false);
}

async function fetchFiles() {
  const project = projects.value.find((item) => item.id === expandedId.value);
  if (!project) return;
  filesLoading.value = true;
  filesErr.value = "";
  try {
    files.value = await api.listMarketFiles(
      source.value,
      project.id,
      kind.value,
      gameVersion.value,
      loader.value
    );
    if (files.value.length === 0) {
      filesErr.value = "这个筛选条件下没有文件，把游戏版本或加载器放宽再试。";
    }
  } catch (err) {
    filesErr.value = errText(err);
    files.value = [];
  } finally {
    filesLoading.value = false;
  }
}

async function toggle(project: MarketProject) {
  if (expandedId.value === project.id) {
    expandedId.value = null;
    files.value = [];
    filesErr.value = "";
    return;
  }
  expandedId.value = project.id;
  files.value = [];
  await fetchFiles();
}

async function install(file: MarketFile) {
  if (busyName.value) return;
  if (!isModpack.value && !targetInstance.value) {
    emit("notify", "先在上面选一个要装进去的实例", "err");
    return;
  }
  busyName.value = file.name;
  progress.value = { stage: "准备下载", done: 0, total: 0 };
  try {
    if (isModpack.value) {
      const report = await api.installMarketModpack(file);
      emit("notify", `整合包已导入为实例「${report.instance.name}」`, "ok");
      await loadInstances();
      // 新实例建好后直接选中它，并把「我的实例」一起刷新，用户不用手动刷新页面。
      targetInstance.value = report.instance.id;
      emit("instances-changed");
    } else {
      const report = await api.installMarketFile(targetInstance.value, file);
      emit(
        "notify",
        report.skipped
          ? `${report.fileName} 本地已有且校验通过`
          : `${report.fileName} 已装到 ${report.folder}/`,
        "ok"
      );
      // 实例内容变了，外层实例卡片上的统计也要跟着更新。
      emit("instances-changed");
    }
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    busyName.value = null;
    progress.value = null;
  }
}

function sizeText(size: number | null) {
  if (!size) return "";
  const mb = size / 1024 / 1024;
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.max(1, Math.round(size / 1024))} KB`;
}

function countText(total: number) {
  return total >= 10000 ? `${(total / 10000).toFixed(1)} 万` : String(total);
}

watch([kind, source], () => {
  void search();
});

watch(query, () => {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(() => void search(), 400);
});

// 筛选条件变了，把已经展开的项目按新条件重拉一遍。
watch([gameVersion, loader], () => {
  if (expandedId.value) void fetchFiles();
});

onMounted(async () => {
  await loadInstances();
  await search();
  unlisten = await api.onInstallProgress((payload) => {
    progress.value = payload;
  });
});

onUnmounted(() => {
  window.clearTimeout(debounceTimer);
  unlisten?.();
});
</script>

<template>
  <div class="market">
    <div class="toolbar">
      <div class="chips">
        <button
          v-for="item in kinds"
          :key="item.key"
          class="chip"
          :class="{ active: kind === item.key }"
          @click="kind = item.key"
        >
          {{ item.label }}
        </button>
      </div>
      <div class="chips">
        <button
          v-for="item in sources"
          :key="item.key"
          class="chip alt"
          :class="{ active: source === item.key }"
          @click="source = item.key"
        >
          {{ item.label }}
        </button>
      </div>
      <input
        v-model="query"
        class="search"
        :placeholder="`搜索${kindLabel}`"
      />
    </div>

    <div class="filters">
      <label class="field">
        <span>版本</span>
        <input v-model="gameVersion" placeholder="不限" />
      </label>
      <label class="field">
        <span>加载器</span>
        <select v-model="loader">
          <option v-for="item in loaders" :key="item.key" :value="item.key">{{ item.label }}</option>
        </select>
      </label>
      <label v-if="!isModpack" class="field grow">
        <span>装到</span>
        <select v-model="targetInstance">
          <option v-if="instances.length === 0" value="">还没有实例</option>
          <option v-for="item in instances" :key="item.id" :value="item.id">
            {{ item.name }}
          </option>
        </select>
      </label>
      <span v-else class="field-note">整合包 → 新建独立实例</span>
    </div>

    <div v-if="progress" class="progress-panel panel">
      <div class="progress-head">
        <strong>{{ busyName }}</strong>
        <span>{{ progress.stage }}<template v-if="progress.total"> · {{ percent }}%</template></span>
      </div>
      <div class="bar">
        <div class="bar-fill" :style="{ width: `${percent}%` }" />
      </div>
    </div>

    <div v-if="loadErr" class="notice err">
      <span>{{ loadErr }}</span>
      <button @click="search()">重试</button>
    </div>

    <div v-if="loading && projects.length === 0" class="skeleton-list">
      <div v-for="n in 5" :key="n" class="skeleton panel" />
    </div>

    <div class="list">
      <article
        v-for="project in projects"
        :key="`${project.source}-${project.id}`"
        class="panel item"
      >
        <div class="item-head" @click="toggle(project)">
          <img v-if="project.icon" class="icon" :src="project.icon" alt="" loading="lazy" />
          <div v-else class="icon placeholder">{{ project.name.slice(0, 1) }}</div>
          <div class="item-main">
            <h2>
              {{ project.name }}
            </h2>
            <p>{{ project.summary || "没有简介" }}</p>
            <p class="meta">
              <span v-if="project.author">{{ project.author }}</span>
              <span>{{ countText(project.downloads) }} 次下载</span>
              <span class="expand">{{ expandedId === project.id ? "收起文件" : "选择文件" }}</span>
            </p>
          </div>
        </div>

        <div v-if="expandedId === project.id" class="files">
          <p v-if="filesLoading" class="state">正在取文件列表…</p>
          <p v-else-if="filesErr" class="state err">{{ filesErr }}</p>
          <div v-for="file in files" :key="file.url" class="file-row">
            <div class="file-main">
              <strong>{{ file.version }}</strong>
              <p>
                <span v-for="gv in file.gameVersions.slice(0, 3)" :key="gv" class="tag muted">{{ gv }}</span>
                <span v-for="ld in file.loaders" :key="ld" class="tag">{{ ld }}</span>
                <span v-if="sizeText(file.size)" class="size">{{ sizeText(file.size) }}</span>
              </p>
            </div>
            <button class="primary" :disabled="!!busyName" @click="install(file)">
              {{ busyName === file.name ? "处理中…" : isModpack ? "下载并导入" : "安装到实例" }}
            </button>
          </div>
        </div>
      </article>
    </div>

    <div v-if="hasMore" class="more">
      <button :disabled="loading" @click="loadMore">{{ loading ? "加载中…" : "加载更多" }}</button>
    </div>

    <p class="foot">
      下载均带完整性校验，本地已有会跳过。模组进实例的 <code>mods/</code>，资源包进
      <code>resourcepacks/</code>，光影进 <code>shaderpacks/</code>，各实例互不干扰。
    </p>
  </div>
</template>

<style scoped>
.market {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.filters {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.chips {
  display: inline-flex;
  gap: 6px;
  padding: 3px;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: var(--radius-md);
}

.chip {
  padding: 5px 12px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
  background: transparent;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}

.chip.active {
  color: #d9ffd3;
  background: linear-gradient(180deg, rgba(98, 204, 85, 0.32), rgba(47, 122, 51, 0.34));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.16);
}

.chip.alt.active {
  color: #cfe8ff;
  background: linear-gradient(180deg, rgba(79, 224, 208, 0.26), rgba(30, 96, 110, 0.36));
}

.search {
  flex: 1;
  min-width: 200px;
  max-width: 360px;
}

.field {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  padding: 4px 8px 4px 10px;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: var(--radius-md);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05);
}

.field.grow {
  flex: 1;
}

.field > span {
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.6px;
  color: var(--text-faint);
  flex-shrink: 0;
}

.field input,
.field select {
  appearance: none;
  -webkit-appearance: none;
  min-width: 0;
  padding: 3px 4px;
  font-size: 12px;
  color: var(--text);
  background: transparent;
  border: none;
  outline: none;
}

.field input::placeholder {
  color: var(--text-faint);
}

.field select {
  padding-right: 16px;
  cursor: pointer;
  background-image: linear-gradient(45deg, transparent 50%, var(--text-faint) 50%),
    linear-gradient(135deg, var(--text-faint) 50%, transparent 50%);
  background-position: right 6px center, right 2px center;
  background-size: 4px 4px, 4px 4px;
  background-repeat: no-repeat;
  text-overflow: ellipsis;
}

.field select option {
  background: #121a22;
  color: var(--text);
}

.field:focus-within {
  border-color: rgba(98, 204, 85, 0.5);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05), 0 0 0 1px rgba(98, 204, 85, 0.2);
}

.tip {
  font-size: 11.5px;
  color: var(--text-faint);
}

/* 与 .field 同高度的静态说明块，避免裸文本和控件并排时出现割裂感 */
.field-note {
  display: inline-flex;
  align-items: center;
  padding: 8px 12px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.4px;
  color: var(--text-faint);
  background: #0c1218;
  border: 2px solid #0a0e12;
  border-radius: var(--radius-md);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}

.progress-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.progress-head {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-dim);
}

.bar {
  height: 6px;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: 3px;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  background: linear-gradient(180deg, var(--grass-hi), var(--grass-deep));
  transition: width 0.25s ease;
}

.notice {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 10px 14px;
  font-size: 12px;
  border-radius: var(--radius-md);
  border: 2px solid rgba(240, 180, 41, 0.35);
  background: rgba(240, 180, 41, 0.08);
  color: #ffe3a8;
}

.notice.err {
  border-color: rgba(255, 106, 106, 0.4);
  background: rgba(255, 106, 106, 0.08);
  color: #ffc9c9;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.skeleton {
  height: 72px;
  opacity: 0.32;
  animation: pulse 1.4s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 0.18;
  }
  50% {
    opacity: 0.4;
  }
}

.item {
  padding: 12px 16px;
}

.item-head {
  display: flex;
  gap: 12px;
  cursor: pointer;
}

.icon {
  width: 46px;
  height: 46px;
  border-radius: 4px;
  border: 2px solid #0a0e12;
  object-fit: cover;
  background: #0f151b;
  flex-shrink: 0;
}

.icon.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  font-weight: 800;
  color: var(--text-faint);
}

.item-main {
  flex: 1;
  min-width: 0;
}

.item-main h2 {
  margin: 0 0 3px;
  font-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.item-main p {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-faint);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-main .meta {
  display: flex;
  gap: 12px;
  margin-top: 4px;
}

.expand {
  color: var(--diamond);
}

.files {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.state {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-faint);
}

.state.err {
  color: #ffbcbc;
}

.file-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 8px 10px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
}

.file-main {
  min-width: 0;
}

.file-main strong {
  font-size: 12.5px;
}

.file-main p {
  margin: 3px 0 0;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.size {
  font-size: 11px;
  color: var(--text-faint);
}

.more {
  display: flex;
  justify-content: center;
  margin-top: 6px;
}

.foot {
  margin: 8px 0 0;
  font-size: 11px;
  line-height: 1.8;
  color: var(--text-faint);
}

.foot code {
  background: #0f151b;
  border: 1px solid #0a0e12;
  border-radius: 3px;
  padding: 1px 5px;
}
</style>
