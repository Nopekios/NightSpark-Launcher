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

    <div class="panel block">
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
          <span class="check">{{ selected === item.path ? "已选" : "" }}</span>
        </li>
      </ul>
    </div>

    <div class="panel block">
      <div class="block-head">
        <h2>一键装 Java</h2>
        <span class="tag muted">Mojang 官方运行时</span>
      </div>
      <p class="hint">
        没合适版本的 Java 时在这儿下一个，装到 %APPDATA%\Love-Launcher\runtimes\，下载进度看底部进度条。
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

    <div class="panel block">
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

    <div class="panel block">
      <div class="block-head">
        <h2>目录</h2>
      </div>
      <ul class="kv">
        <li>
          <span>账号缓存</span>
          <em>%APPDATA%\Love-Launcher\account.json</em>
        </li>
        <li>
          <span>实例数据</span>
          <em>%APPDATA%\Love-Launcher\instances\</em>
        </li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.runtime-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.runtime-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 12px 14px;
  border-radius: var(--radius-md, 12px);
  border: 1px solid var(--line);
  background: var(--bg-soft, rgba(255, 255, 255, 0.02));
}

.rt-main {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  font-size: 12.5px;
}

.rt-main strong {
  font-size: 13px;
}

.rt-note {
  color: var(--text-dim);
}
.head-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

.block {
  margin-bottom: 18px;
}

.block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

.block-head h2 {
  margin: 0;
  font-size: 15px;
}

.hint {
  margin: 8px 0 0;
  font-size: 12px;
  line-height: 1.7;
  color: var(--text-faint);
}

.error {
  margin: 8px 0 0;
  font-size: 12px;
  color: #ffb4b4;
}

.java-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.java-list li {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 11px 14px;
  border-radius: var(--radius-md);
  border: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.02);
  cursor: pointer;
  transition: border-color 0.18s ease, background 0.18s ease;
}

.java-list li:hover {
  border-color: var(--line-strong);
  background: rgba(255, 255, 255, 0.05);
}

.java-list li.picked {
  border-color: rgba(90, 216, 111, 0.45);
  background: rgba(90, 216, 111, 0.08);
}

.java-ver {
  flex: 0 0 52px;
  text-align: center;
  padding: 5px 0;
  border-radius: var(--radius-sm);
  font-size: 12px;
  font-weight: 700;
  color: #06120a;
  background: rgba(90, 216, 111, 0.85);
}

.java-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
}

.java-meta .path {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.java-meta em {
  font-size: 11px;
  font-style: normal;
  color: var(--text-faint);
}

.check {
  font-size: 11px;
  color: var(--grass);
}

.memory {
  display: flex;
  align-items: center;
  gap: 16px;
}

.memory input[type="range"] {
  flex: 1;
  appearance: none;
  height: 10px;
  border-radius: 3px;
  padding: 0;
  border: 2px solid #0a0e12;
  background: #0f151b;
  box-shadow: inset 0 2px 6px rgba(0, 0, 0, 0.55);
}

.memory input[type="range"]::-webkit-slider-thumb {
  appearance: none;
  width: 16px;
  height: 20px;
  border-radius: 3px;
  background: linear-gradient(180deg, var(--grass-hi), var(--grass-deep));
  border: 2px solid #14351a;
  box-shadow: inset 0 2px 0 rgba(255, 255, 255, 0.3), 0 2px 0 rgba(0, 0, 0, 0.45);
  cursor: pointer;
}

.memory strong {
  flex: 0 0 62px;
  text-align: right;
  font-size: 14px;
}

.kv {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.kv li {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  font-size: 12px;
}

.kv span {
  color: var(--text-dim);
}

.kv em {
  font-style: normal;
  color: var(--text-faint);
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
}

/* ===== 设置页分区强化 ===== */
.block {
  margin-bottom: 14px;
}

.block-head h2 {
  margin: 0;
  font-size: 14px;
  font-weight: 800;
  letter-spacing: 0.4px;
}

.block-head h2::before {
  content: "";
  display: inline-block;
  width: 4px;
  height: 13px;
  margin-right: 9px;
  vertical-align: -2px;
  background: var(--grass);
  box-shadow: 0 0 8px rgba(98, 204, 85, 0.6);
}

.java-list li {
  border: 2px solid #0b1015;
  border-radius: 4px;
  margin-bottom: 8px;
}

.java-list li.picked {
  border-color: rgba(98, 204, 85, 0.8);
  background: rgba(98, 204, 85, 0.08);
  box-shadow: 0 0 0 2px rgba(98, 204, 85, 0.14);
}

input[type="range"] {
  -webkit-appearance: none;
  appearance: none;
  height: 12px;
  padding: 0;
  background: #0f151b;
  border: 2px solid #0a0e12;
  border-radius: 3px;
  box-shadow: inset 0 2px 6px rgba(0, 0, 0, 0.55);
}

input[type="range"]::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 18px;
  height: 22px;
  border-radius: 3px;
  background: linear-gradient(180deg, var(--grass-hi), var(--grass-deep));
  border: 2px solid #14351a;
  box-shadow: inset 0 2px 0 rgba(255, 255, 255, 0.3), 0 2px 0 rgba(0, 0, 0, 0.45);
  cursor: pointer;
}

.kv li {
  padding: 9px 0;
  border-bottom: 1px dashed rgba(255, 255, 255, 0.07);
}

.kv li:last-child {
  border-bottom: none;
}

</style>
