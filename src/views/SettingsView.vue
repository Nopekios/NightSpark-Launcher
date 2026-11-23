<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  api,
  errText,
  type JavaInfo,
  type JavaRuntime,
  type LauncherConfig,
} from "../api";

const emit = defineEmits<{
  (e: "notify", text: string, tone?: "ok" | "err"): void;
}>();

const javas = ref<JavaInfo[]>([]);
const scanning = ref(false);
const selected = ref<string | null>(null);
const tab = ref<"runtime" | "install" | "memory" | "paths">("runtime");
const scanError = ref("");
const memory = ref(4096);
const saving = ref(false);
const saved = ref(false);
const cfgRef = ref<LauncherConfig>({ java_path: null, memory_mb: 4096, game_dir: null });

// 启动器自己下的 Java（绕开系统里没装低版本 Java 的坑）。
const runtimes = ref<JavaRuntime[]>([]);
const installingMajor = ref<number | null>(null);
const MAJOR_HINTS = [
  { major: 8, note: "1.16.5 及以下的老版本" },
  { major: 17, note: "1.17 ~ 1.20.4" },
  { major: 21, note: "1.20.5 及以上" },
];

function runtimeOf(major: number): JavaRuntime | null {
  return runtimes.value.find((item) => item.major === major) ?? null;
}

async function loadRuntimes() {
  try {
    runtimes.value = await api.listJavaRuntimes();
  } catch {
    runtimes.value = [];
  }
}

async function installJava(major: number) {
  if (installingMajor.value) return;
  installingMajor.value = major;
  try {
    const runtime = await api.installJava(major);
    emit("notify", `Java ${runtime.major} 装好了：${runtime.path}`);
    await Promise.all([loadRuntimes(), scan()]);
    if (!selected.value) selected.value = runtime.path;
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    installingMajor.value = null;
  }
}

async function scan() {
  if (scanning.value) return;
  scanning.value = true;
  scanError.value = "";
  try {
    javas.value = await api.javaList();
    if (javas.value.length > 0 && !selected.value) {
      selected.value = javas.value[0].path;
    }
  } catch (err) {
    scanError.value = errText(err);
  } finally {
    scanning.value = false;
  }
}

async function loadConfig() {
  try {
    const cfg = await api.getConfig();
    cfgRef.value = cfg;
    memory.value = cfg.memory_mb;
    if (cfg.java_path) selected.value = cfg.java_path;
  } catch (err) {
    scanError.value = errText(err);
  }
}

async function save() {
  if (saving.value) return;
  saving.value = true;
  saved.value = false;
  try {
    const next: LauncherConfig = {
      ...cfgRef.value,
      java_path: selected.value,
      memory_mb: memory.value,
    };
    await api.saveConfig(next);
    cfgRef.value = next;
    saved.value = true;
    emit("notify", "启动设置已保存");
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    saving.value = false;
  }
}

onMounted(async () => {
  await Promise.all([scan(), loadConfig(), loadRuntimes()]);
});
</script>

<template>
  <section class="page">
    <header class="page-head">
      <div>
        <h1>设置</h1>
        <p>Java 运行时自动扫描，选定后直接用于组装启动命令行。</p>
      </div>
      <div class="head-actions">
        <button :disabled="scanning" @click="scan">{{ scanning ? "扫描中…" : "重新扫描" }}</button>
        <button class="primary" :disabled="saving" @click="save">
          {{ saving ? "保存中…" : saved ? "已保存" : "保存设置" }}
        </button>
      </div>
    </header>

    <div class="set-shell">
      <nav class="set-nav">
        <button :class="{ on: tab === 'runtime' }" @click="tab = 'runtime'">
          <strong>运行时</strong><span>Java 选择</span>
        </button>
        <button :class="{ on: tab === 'install' }" @click="tab = 'install'">
          <strong>一键装</strong><span>官方运行时</span>
        </button>
        <button :class="{ on: tab === 'memory' }" @click="tab = 'memory'">
          <strong>内存</strong><span>Xmx 分配</span>
        </button>
        <button :class="{ on: tab === 'paths' }" @click="tab = 'paths'">
          <strong>目录</strong><span>缓存与实例</span>
        </button>
      </nav>
      <div class="set-panes">
    <Transition name="pane" mode="out-in">
      <div v-if="tab === 'runtime'" class="panel block">
      <div class="block-head">
        <h2>Java 运行时</h2>
        <span class="tag muted">{{ javas.length }} 个候选</span>
      </div>
      <p v-if="scanError" class="error">{{ scanError }}</p>
      <p v-else-if="javas.length === 0 && !scanning" class="hint">
        没扫到 Java。装一个 JDK 17 / 21（Temurin 即可），或者把 java.exe 加进 PATH。
      </p>
      <ul v-else class="java-list">
        <li
          v-for="item in javas"
          :key="item.path"
          :class="{ picked: selected === item.path }"
          @click="selected = item.path"
        >
          <div class="java-ver">{{ item.version }}</div>
          <div class="java-meta">
            <span class="path" :title="item.path">{{ item.path }}</span>
            <em>Java {{ item.version }}</em>
          </div>
          <span class="check">{{ selected === item.path ? "已选择" : "" }}</span>
        </li>
      </ul>
    </div>

    <div v-else-if="tab === 'install'" class="panel block">
      <div class="block-head">
        <h2>一键装 Java</h2>
        <span class="tag muted">Mojang 官方运行时</span>
      </div>
      <p class="hint">
        没合适版本的 Java 时在这儿下一个，装到 %APPDATA%\NightSpark\runtimes\，下载进度看底部进度条。
      </p>
      <ul class="runtime-list">
        <li v-for="item in MAJOR_HINTS" :key="item.major">
          <div class="rt-main">
            <strong>Java {{ item.major }}</strong>
            <span class="rt-note">{{ item.note }}</span>
            <span v-if="runtimeOf(item.major)" class="tag ok">
              已装 · {{ runtimeOf(item.major)?.version }}
            </span>
          </div>
          <button :disabled="!!installingMajor" @click="installJava(item.major)">
            {{
              installingMajor === item.major
                ? "下载中…"
                : runtimeOf(item.major)
                  ? "重新下载"
                  : "下载安装"
            }}
          </button>
        </li>
      </ul>
    </div>

    <div v-else-if="tab === 'memory'" class="panel block">
      <div class="block-head">
        <h2>内存分配</h2>
        <span class="tag ok">生效中</span>
      </div>
      <div class="memory">
        <input v-model.number="memory" type="range" min="1024" max="16384" step="512" />
        <strong>{{ (memory / 1024).toFixed(1) }} GB</strong>
      </div>
      <p class="hint">
        取值范围 1–16 GB，启动时作为 -Xmx 传给 JVM；带整合包建议 6 GB 以上。
      </p>
    </div>

    <div v-else class="panel block">
      <div class="block-head">
        <h2>目录</h2>
      </div>
      <ul class="kv">
        <li>
          <span>账号缓存</span>
          <em>%APPDATA%\NightSpark\account.json</em>
        </li>
        <li>
          <span>实例数据</span>
          <em>%APPDATA%\NightSpark\instances\</em>
        </li>
      </ul>
    </div>
    </Transition>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* 分区切换：分页式横移 + 淡入（替代原来的瞬切） */
.pane-enter-active {
  transition: opacity 0.26s ease, transform 0.32s cubic-bezier(0.22, 1, 0.36, 1);
}

.pane-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}

.pane-enter-from {
  opacity: 0;
  transform: translateX(18px);
}

.pane-leave-to {
  opacity: 0;
  transform: translateX(-14px);
}

.head-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* ── 分区 ── */
.block {
  padding: 22px 24px;
  margin-bottom: 16px;
}

.block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  margin-bottom: 14px;
  padding-bottom: 12px;
  border-bottom: 1px solid rgba(126, 160, 199, 0.1);
}

.block-head h2 {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 0;
  font-size: 14px;
  font-weight: 700;
  letter-spacing: -0.01em;
}

.block-head h2::before {
  content: "";
  width: 3px;
  height: 13px;
  border-radius: 999px;
  background: linear-gradient(180deg, #3eb1ff, #0072c6);
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.85);
}

.hint {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.7;
  color: var(--text-faint);
}

.error {
  margin: 12px 0 0;
  padding: 11px 14px;
  border-radius: 11px;
  border: 1px solid rgba(255, 90, 110, 0.34);
  background: rgba(255, 90, 110, 0.07);
  font-size: 12px;
  color: #ffc2ca;
}

/* ── Java 运行时列表 ── */
.runtime-list,
.java-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.runtime-list li,
.java-list li {
  display: flex;
  overflow: hidden;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 13px 16px;
  border-radius: 12px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.03) 0%, rgba(255, 255, 255, 0.007) 100%),
    rgba(12, 18, 27, 0.8);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  font-size: 12.5px;
  transition: border-color 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.22s cubic-bezier(0.22, 1, 0.36, 1), transform 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    background 0.22s ease;
}

.java-list li {
  cursor: pointer;
}

.java-list li:hover {
  border-color: rgba(0, 149, 255, 0.3);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 16px 34px -22px rgba(0, 149, 255, 0.85);
  transform: translateY(-1px);
}

.java-list li.picked {
  border-color: rgba(0, 149, 255, 0.48);
  background: linear-gradient(100deg, rgba(0, 149, 255, 0.16) 0%, rgba(0, 149, 255, 0.03) 64%),
    rgba(12, 18, 27, 0.9);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 0 0 1px rgba(0, 149, 255, 0.12),
    0 18px 40px -24px rgba(0, 149, 255, 0.95);
}

.rt-main {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.rt-main strong {
  font-size: 12.5px;
  font-weight: 650;
  letter-spacing: -0.005em;
  color: var(--text);
}

.rt-note {
  font-size: 11px;
  color: var(--text-faint);
}

.java-ver {
  flex: none;
  padding: 3px 10px;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.03em;
  color: #bfe0ff;
  background: rgba(0, 149, 255, 0.12);
  border: 1px solid rgba(0, 149, 255, 0.22);
}

.java-meta {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1;
}

.java-meta .path {
  display: block;
  max-width: 100%;
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
  color: #7f95ab;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.java-meta em {
  display: block;
  max-width: 100%;
  font-style: normal;
  font-size: 10.5px;
  color: var(--text-faint);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.check {
  display: grid;
  place-items: center;
  width: 20px;
  height: 20px;
  flex: none;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 800;
  color: #05121c;
  background: linear-gradient(180deg, #6fd0ff, #0095ff);
  box-shadow: 0 0 12px rgba(0, 149, 255, 0.7);
}

/* ── 内存 ── */
.memory {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.memory strong {
  font-size: 21px;
  font-weight: 700;
  letter-spacing: -0.03em;
  font-variant-numeric: tabular-nums;
  color: #cfe8ff;
}

/* ── 键值信息 ── */
.kv {
  display: flex;
  flex-direction: column;
  gap: 0;
  margin: 0;
  padding: 0;
  list-style: none;
}

.kv li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 10px 2px;
  border-bottom: 1px solid rgba(126, 160, 199, 0.08);
  font-size: 12px;
}

.kv li:last-child {
  border-bottom: none;
}

.kv span {
  color: var(--text-dim);
}

.kv em {
  font-style: normal;
  font-weight: 600;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
/* ── 左侧分类导航 + 右侧单分区 ── */
.set-shell {
  display: grid;
  grid-template-columns: 208px minmax(0, 1fr);
  gap: 18px;
  align-items: start;
}

.set-nav {
  position: sticky;
  top: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px;
  border-radius: 16px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: linear-gradient(180deg, rgba(18, 28, 40, 0.74), rgba(10, 15, 23, 0.74));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}

.set-nav button {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 10px 12px;
  border: none;
  border-radius: 10px;
  background: none;
  box-shadow: none;
  text-align: left;
  cursor: pointer;
  transition: background 0.24s ease;
}

.set-nav button:hover {
  transform: none;
  box-shadow: none;
  border-color: transparent;
  background: rgba(126, 160, 199, 0.08);
}

.set-nav button::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  width: 2px;
  height: 0;
  border-radius: 999px;
  background: linear-gradient(180deg, #3eb1ff, #0072c6);
  transform: translateY(-50%);
  transition: height 0.28s cubic-bezier(0.22, 1, 0.36, 1);
}

.set-nav button strong {
  font-size: 12.5px;
  font-weight: 650;
  color: var(--text-dim);
}

.set-nav button span {
  font-size: 10.5px;
  color: var(--text-faint);
}

.set-nav button.on {
  background: rgba(0, 149, 255, 0.1);
}

.set-nav button.on::before {
  height: 62%;
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.9);
}

.set-nav button.on strong {
  color: #dff0ff;
}

.set-panes {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.set-panes .block {
  margin-bottom: 0;
  padding: 20px 22px;
}

/* 选中项左侧光条 */
.java-list li {
  position: relative;
}

.java-list li.picked::after {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: linear-gradient(180deg, #3eb1ff, #0072c6);
  box-shadow: 0 0 12px rgba(0, 149, 255, 0.9);
}

/* 已选择徽章：改成 pill，不再被 20px 圆点裁掉文字 */
.check {
  display: grid;
  place-items: center;
  width: auto;
  min-width: 0;
  height: 22px;
  padding: 0 10px;
  flex: none;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.02em;
  white-space: nowrap;
  color: #05121c;
  background: linear-gradient(180deg, #6fd0ff, #0095ff);
  box-shadow: 0 0 14px rgba(0, 149, 255, 0.6);
}

.check:empty {
  display: none;
}

</style>
