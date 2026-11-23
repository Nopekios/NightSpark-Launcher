<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { currentLocale } from "../i18n";
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
const { t } = useI18n();

const kinds = computed(() => [
  { key: "mod", label: t("market.kindMod") },
  { key: "modpack", label: t("market.kindModpack") },
  { key: "resourcepack", label: t("market.kindResourcepack") },
  { key: "shader", label: t("market.kindShader") },
]);

/** 两条来源：CF 走镜像、Modrinth 走官方。 */
const sources = [
  { key: "curseforge", label: "CurseForge" },
  { key: "modrinth", label: "Modrinth" },
] as const;

const loaders = computed(() => [
  { key: "", label: t("market.any") },
  { key: "forge", label: "Forge" },
  { key: "neoforge", label: "NeoForge" },
  { key: "fabric", label: "Fabric" },
  { key: "quilt", label: "Quilt" },
]);

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
const kindLabel = computed(
  () => kinds.value.find((item) => item.key === kind.value)?.label ?? t("market.kindFallback"),
);
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
    emit("notify", t("market.instanceReadFail", { reason: errText(err) }), "err");
  }
}

// 请求序号：切换分类/来源/搜索词时网络响应可能乱序返回，
// 用序号把上一轮迟到的结果丢掉，避免选着「整合包」却显示「光影」的列表。
let searchSeq = 0;

async function search(reset = true) {
  if (reset) {
    page.value = 0;
    projects.value = [];
    expandedId.value = null;
    files.value = [];
  }
  const seq = ++searchSeq;
  loading.value = true;
  loadErr.value = "";
  try {
    const list = await api.searchMarket(source.value, kind.value, query.value, page.value);
    if (seq !== searchSeq) return;
    projects.value = reset ? list : [...projects.value, ...list];
    hasMore.value = list.length >= PAGE_SIZE;
    if (reset && list.length === 0) {
      loadErr.value = query.value.trim()
        ? t("market.empty")
        : t("market.emptySource");
    }
  } catch (err) {
    if (seq !== searchSeq) return;
    loadErr.value = errText(err);
    hasMore.value = false;
  } finally {
    if (seq === searchSeq) loading.value = false;
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
      filesErr.value = t("market.filesEmpty");
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
    emit("notify", t("market.pickInstance"), "err");
    return;
  }
  busyName.value = file.name;
  progress.value = { stage: t("market.stagePreparing"), done: 0, total: 0 };
  try {
    if (isModpack.value) {
      const report = await api.installMarketModpack(file);
      emit("notify", t("market.packImported", { name: report.instance.name }), "ok");
      await loadInstances();
      // 新实例建好后直接选中它，并把「我的实例」一起刷新，用户不用手动刷新页面。
      targetInstance.value = report.instance.id;
      emit("instances-changed");
    } else {
      const report = await api.installMarketFile(targetInstance.value, file);
      emit(
        "notify",
        report.skipped
          ? t("market.fileExists", { file: report.fileName })
          : t("market.fileInstalled", { file: report.fileName, folder: report.folder }),
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
  if (currentLocale() === "en-US") return total.toLocaleString("en-US");
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
        :placeholder="t('market.searchPh', { kind: kindLabel })"
      />
    </div>

    <div class="filters">
      <label class="field">
        <span>{{ t("market.versionField") }}</span>
        <input v-model="gameVersion" :placeholder="t('market.any')" />
      </label>
      <label class="field">
        <span>{{ t("market.loaderField") }}</span>
        <select v-model="loader">
          <option v-for="item in loaders" :key="item.key" :value="item.key">{{ item.label }}</option>
        </select>
      </label>
      <label v-if="!isModpack" class="field grow">
        <span>{{ t("market.targetField") }}</span>
        <select v-model="targetInstance">
          <option v-if="instances.length === 0" value="">{{ t("market.noInstances") }}</option>
          <option v-for="item in instances" :key="item.id" :value="item.id">
            {{ item.name }}
          </option>
        </select>
      </label>
      <span v-else class="field-note">{{ t("market.modpackNote") }}</span>
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
      <button @click="search()">{{ t("common.retry") }}</button>
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
            <p>{{ project.summary || t("market.noSummary") }}</p>
            <p class="meta">
              <span v-if="project.author">{{ project.author }}</span>
              <span>{{ t("market.downloads", { n: countText(project.downloads) }) }}</span>
              <span class="expand">{{ expandedId === project.id ? t("market.collapse") : t("market.expand") }}</span>
            </p>
          </div>
        </div>

        <div v-if="expandedId === project.id" class="files">
          <p v-if="filesLoading" class="state">{{ t("market.filesLoading") }}</p>
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
              {{ busyName === file.name ? t("market.busy") : isModpack ? t("market.downloadImport") : t("market.installToInstance") }}
            </button>
          </div>
        </div>
      </article>
    </div>

    <div v-if="hasMore" class="more">
      <button :disabled="loading" @click="loadMore">{{ loading ? t("market.loadingMore") : t("market.loadMore") }}</button>
    </div>

    <p class="foot">
      {{ t("market.footerA") }}<code>mods/</code>{{ t("market.footerB") }}<code>resourcepacks/</code>{{ t("market.footerC") }}<code>shaderpacks/</code>{{ t("market.footerD") }}
    </p>
  </div>
</template>

<style scoped>
.market {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

/* ── 工具条 ── */
.toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}

.filters {
  display: flex;
  align-items: center;
  gap: 6px;
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip {
  padding: 6px 13px;
  border-radius: 999px;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: rgba(255, 255, 255, 0.03);
  box-shadow: none;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-dim);
  transition: color 0.2s ease, border-color 0.2s ease, background 0.2s ease, box-shadow 0.2s ease;
}

.chip:hover:not(.active) {
  color: var(--text);
  border-color: rgba(0, 149, 255, 0.3);
  background: rgba(0, 149, 255, 0.07);
  transform: none;
  box-shadow: none;
}

.chip.active {
  color: #eaf6ff;
  border-color: rgba(0, 149, 255, 0.5);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.28) 0%, rgba(0, 149, 255, 0.1) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.16), 0 8px 20px -14px rgba(0, 149, 255, 0.95);
}

.chip.alt.active {
  border-color: rgba(53, 224, 200, 0.45);
  background: linear-gradient(180deg, rgba(53, 224, 200, 0.24) 0%, rgba(53, 224, 200, 0.08) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.14), 0 8px 20px -14px rgba(53, 224, 200, 0.9);
}

.search {
  flex: 1;
  min-width: 180px;
  padding: 8px 15px;
  border-radius: 999px;
  background: rgba(10, 18, 28, 0.9);
}

.field {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  height: 34px;
  border-radius: 999px;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: rgba(10, 18, 28, 0.9);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.4);
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.field.grow {
  flex: 1;
  min-width: 180px;
}

.field:focus-within {
  border-color: rgba(0, 149, 255, 0.55);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.4), 0 0 0 3px rgba(0, 149, 255, 0.12);
}

.field > span {
  flex: none;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field input,
.field select {
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: 0;
  border: none;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
  font-size: 12px;
}

.field input:focus,
.field select:focus {
  border: none;
  background: transparent;
  box-shadow: none;
}

.field input::placeholder {
  color: var(--text-faint);
}

.field select option {
  background: #0c121b;
  color: var(--text);
}

.tip,
.field-note {
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-faint);
}

/* ── 进度 ── */
.progress-panel {
  padding: 14px 17px;
  border-radius: 13px;
  border: 1px solid rgba(0, 149, 255, 0.24);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.09) 0%, rgba(0, 149, 255, 0.02) 100%),
    rgba(10, 16, 24, 0.9);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 16px 40px -26px rgba(0, 149, 255, 0.85);
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

.notice {
  padding: 12px 15px;
  border-radius: 11px;
  border: 1px solid rgba(126, 160, 199, 0.16);
  background: rgba(126, 160, 199, 0.06);
  font-size: 12px;
  color: var(--text-dim);
}

.notice.err {
  border-color: rgba(255, 90, 110, 0.34);
  background: rgba(255, 90, 110, 0.07);
  color: #ffc2ca;
}

/* ── 资源卡片 ── */
.list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.item {
  padding: 16px 18px;
  border-radius: 14px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.035) 0%, rgba(255, 255, 255, 0.008) 100%),
    rgba(12, 18, 27, 0.82);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  transition: border-color 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.22s cubic-bezier(0.22, 1, 0.36, 1), transform 0.22s cubic-bezier(0.22, 1, 0.36, 1);
}

.item:hover {
  border-color: rgba(0, 149, 255, 0.3);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 20px 42px -26px rgba(0, 149, 255, 0.85);
  transform: translateY(-1px);
}

.item-head {
  display: flex;
  align-items: flex-start;
  gap: 13px;
}

.icon {
  width: 40px;
  height: 40px;
  flex: none;
  border-radius: 11px;
  object-fit: cover;
  border: 1px solid rgba(126, 160, 199, 0.16);
  background: rgba(126, 160, 199, 0.06);
}

.icon.placeholder {
  display: grid;
  place-items: center;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-faint);
  background: linear-gradient(150deg, rgba(0, 149, 255, 0.14), rgba(126, 160, 199, 0.05));
}

.item-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.item-main h2 {
  margin: 0;
  font-size: 13.5px;
  font-weight: 650;
  letter-spacing: -0.005em;
  color: var(--text);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.item-main p {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.62;
  color: var(--text-dim);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.item-main .meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 2px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-faint);
}

.expand {
  flex: none;
  padding: 7px 15px;
  border-radius: 999px;
  font-size: 11.5px;
}

.files {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 13px;
  padding-top: 13px;
  border-top: 1px solid rgba(126, 160, 199, 0.1);
  animation: ll-fade-up 0.24s cubic-bezier(0.22, 1, 0.36, 1) both;
}

.file-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 9px 12px;
  border-radius: 9px;
  background: rgba(0, 0, 0, 0.22);
  font-size: 11.5px;
}

.file-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.file-main strong {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.file-main p {
  margin: 0;
  font-size: 10.5px;
  color: var(--text-faint);
}

.size {
  flex: none;
  font-size: 10.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
}

.state {
  font-size: 11.5px;
  font-weight: 650;
  color: var(--text-faint);
}

.state.err {
  color: #ff9daa;
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.skeleton {
  height: 88px;
}

.more {
  align-self: center;
  margin-top: 4px;
  padding: 9px 22px;
  border-radius: 999px;
}

.foot {
  margin: 4px 0 0;
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
</style>
