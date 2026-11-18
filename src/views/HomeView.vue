<script setup lang="ts">
import { computed, ref } from "vue";
import type { GameInstance } from "../api";

const props = defineProps<{
  instances: GameInstance[];
  busy: boolean;
  hasAccount: boolean;
  launchingId: string | null;
  runningPids: Record<string, number>;
}>();

const emit = defineEmits<{
  (e: "create", payload: { name: string; version: string }): void;
  (e: "remove", item: GameInstance): void;
  (e: "open-account"): void;
  (e: "launch", item: GameInstance): void;
  (e: "show-log", item: GameInstance): void;
  (e: "kill", item: GameInstance): void;
  (e: "import-local"): void;
  (e: "fetch-mods", item: GameInstance): void;
  (e: "install-loader", payload: { item: GameInstance; spec: string }): void;
  (e: "verify", item: GameInstance): void;
  (e: "sync-files", item: GameInstance): void;
}>();

const showCreate = ref(false);
const name = ref("");
const version = ref("1.20.1");
const pendingDelete = ref<string | null>(null);
const query = ref("");
const activeId = ref<string | null>(null);
// 加载器写法，比如 forge-47.2.0；留空就让后端按整合包要求自己挑
const loaderSpec = ref("");

// 先选实例再展开：只有用户点进去的那一个才作为当前对象。
const hero = computed(
  () => props.instances.find((item) => item.id === activeId.value) ?? null
);

function enter(item: GameInstance) {
  activeId.value = item.id;
  pendingDelete.value = null;
}

function backToList() {
  activeId.value = null;
  pendingDelete.value = null;
}

// 当前实例是否已经有进程在跑（决定主按钮走「启动」还是「已运行」）。
const heroPid = computed(() => (hero.value ? props.runningPids[hero.value.id] : undefined));

const filtered = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  if (!keyword) return props.instances;
  return props.instances.filter(
    (item) =>
      item.name.toLowerCase().includes(keyword) || item.version.toLowerCase().includes(keyword)
  );
});

function openCreate() {
  name.value = "";
  version.value = "1.20.1";
  showCreate.value = true;
}

function submit() {
  const trimmedName = name.value.trim();
  const trimmedVersion = version.value.trim();
  if (!trimmedName || !trimmedVersion) return;
  emit("create", { name: trimmedName, version: trimmedVersion });
  showCreate.value = false;
}
</script>

<template>
  <section class="page home-page">
    <header class="page-head">
      <div>
        <h1>{{ hero ? "实例详情" : "我的实例" }}</h1>
        <p v-if="hero">版本 {{ hero.version }}</p>
        <p v-else>
          共 {{ instances.length }} 个实例<span v-if="instances.length > 0">，点一个进去</span>
        </p>
      </div>
      <div class="head-actions">
        <button v-if="hero" @click="backToList">返回列表</button>
        <template v-else>
          <input v-if="instances.length > 5" v-model="query" class="search" placeholder="搜索实例…" />
          <button @click="openCreate">新建实例</button>
        </template>
      </div>
    </header>

    <template v-if="hero">
      <div class="detail panel">
        <div class="detail-main">
          <span class="tag" :class="{ muted: !hasAccount }">
            {{ heroPid ? `运行中 · PID ${heroPid}` : hasAccount ? "准备就绪" : "未登录 · 需绑定微软账号" }}
          </span>
          <h2 :title="hero.name">{{ hero.name }}</h2>
          <div class="detail-meta">
            <span class="chip">{{ hero.version }}</span>
            <span class="chip muted">独立游戏目录</span>
          </div>
          <details class="path-box">
            <summary>游戏目录</summary>
            <code>{{ hero.game_dir }}</code>
          </details>
          <div class="hero-actions">
            <button v-if="!hasAccount" class="primary play-big" @click="emit('open-account')">
              登录微软账号
            </button>
            <button
              v-else
              class="primary play-big"
              :disabled="busy || launchingId === hero.id"
              title="用当前账号启动这个实例"
              @click="emit('launch', hero)"
            >
              {{ launchingId === hero.id ? "启动中…" : heroPid ? "再开一个" : "启动游戏" }}
            </button>
            <button v-if="heroPid" class="ghost-danger" @click="emit('kill', hero)">结束进程</button>
            <details class="adv">
              <summary>更多</summary>
              <div class="adv-body">
                <button :disabled="busy" title="按清单补齐 CurseForge / Modrinth 的 mods"
                  @click="emit('fetch-mods', hero)">
                  补齐 mods
                </button>
                <input
                  v-model="loaderSpec"
                  :disabled="busy"
                  placeholder="加载器，如 forge-47.2.0"
                  title="写加载器名字就行，带版本更准；想装别的版本就自己填"
                />
                <button :disabled="busy || loaderSpec.trim().length === 0"
                  title="下载并安装 Forge / NeoForge / Fabric / Quilt"
                  @click="emit('install-loader', { item: hero, spec: loaderSpec.trim() })">
                  装加载器
                </button>
                <button :disabled="busy"
                  title="版本隔离的整合包把 mods / config / 存档放在 versions/<版本>/ 里，点这里把这些文件补进实例"
                  @click="emit('sync-files', hero)">
                  补本机文件
                </button>
                <button :disabled="busy" title="按版本 json 里的 sha1 核对 client jar 和依赖库"
                  @click="emit('verify', hero)">
                  校验文件
                </button>
              </div>
            </details>
          </div>
        </div>
        <div class="detail-side">
          <template v-if="pendingDelete === hero.id">
            <p class="danger-hint">实例目录会连同 mods、存档一起删掉。</p>
            <button class="ghost-danger" :disabled="busy" @click="emit('remove', hero)">确认删除</button>
            <button :disabled="busy" @click="pendingDelete = null">取消</button>
          </template>
          <button v-else class="ghost-danger del" :disabled="busy" @click="pendingDelete = hero.id">
            删除实例
          </button>
        </div>
      </div>
    </template>

    <template v-else>
      <div v-if="instances.length === 0" class="hero panel">
        <div class="hero-stage">
          <span class="hero-badge">未创建</span>
          <div class="hero-blocks" aria-hidden="true">
            <span class="blk grass" />
            <span class="blk dirt" />
            <span class="blk stone" />
            <span class="blk diamond" />
          </div>
        </div>
        <div class="hero-info">
          <span class="tag muted">{{ hasAccount ? "第一步" : "未登录 · 需绑定微软账号" }}</span>
          <h2>{{ hasAccount ? "还没有游戏实例" : "先绑定微软账号" }}</h2>
          <p class="hero-meta">
            {{
              hasAccount
                ? "新建一个实例，或导入本机已装版本；整合包 zip 直接拖进窗口也能导入。"
                : "正版登录后才能启动游戏；登录完可以导入本机已装版本，或新建实例。"
            }}
          </p>
          <div class="hero-actions">
            <button v-if="!hasAccount" class="primary play-big" @click="emit('open-account')">
              登录微软账号
            </button>
            <button v-else class="primary play-big" @click="emit('import-local')">
              导入本机已装版本
            </button>
            <button v-if="hasAccount" @click="openCreate">新建实例</button>
            <button v-else @click="emit('import-local')">导入本机已装版本</button>
          </div>
        </div>
      </div>

      <div v-else-if="filtered.length === 0" class="empty-state panel">
        <h2>没有匹配的实例</h2>
        <p>换个关键词试试。</p>
      </div>

      <ul v-else class="pick-list">
        <li v-for="item in filtered" :key="item.id">
          <button class="pick-row" @click="enter(item)">
            <span class="dot" :class="{ running: !!runningPids[item.id] }" />
            <span class="pick-name" :title="item.name">{{ item.name }}</span>
            <span class="pick-ver">{{ item.version }}</span>
            <span class="pick-state">{{ runningPids[item.id] ? "运行中" : "" }}</span>
            <span class="pick-arrow">›</span>
          </button>
        </li>
      </ul>
      <p v-if="instances.length > 0" class="drop-hint">
        导入整合包：把 zip 拖进窗口就行，PCL / HMCL 的结构都能认。
      </p>
    </template>

    <Transition name="toast">
      <div v-if="showCreate" class="modal-mask" @click.self="showCreate = false">
        <div class="modal">
          <h2>新建实例</h2>
          <p class="modal-sub">实例名用英文或中文都行，版本号决定后面能装什么整合包。</p>
          <label>
            <span>实例名称</span>
            <input v-model="name" autofocus placeholder="例如：生电 1.20.1" @keyup.enter="submit" />
          </label>
          <label>
            <span>游戏版本</span>
            <input v-model="version" placeholder="例如：1.20.1" @keyup.enter="submit" />
          </label>
          <div class="modal-actions">
            <button @click="showCreate = false">取消</button>
            <button class="primary" :disabled="busy || !name.trim() || !version.trim()" @click="submit">
              创建
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </section>
</template>

<style scoped>
.run-state {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 8px;
  font-size: 11px;
  color: #4ad7c8;
}

.link {
  padding: 0;
  border: none;
  background: none;
  color: var(--text-dim);
  font-size: 11px;
  text-decoration: underline;
  cursor: pointer;
}

.link:hover {
  color: var(--text);
}

.link.danger {
  color: #ff9c9c;
}

.link.danger:hover {
  color: var(--danger);
}

.head-actions {
  display: flex;
  gap: 10px;
  align-items: center;
}

.search {
  width: 180px;
}

.notice {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 20px;
  padding: 16px 20px;
  border-radius: var(--radius-lg);
  border: 1px solid rgba(240, 180, 41, 0.38);
  background: linear-gradient(135deg, rgba(240, 180, 41, 0.16), rgba(240, 180, 41, 0.03));
}

.notice strong {
  font-size: 13px;
  color: #ffdd94;
}

.notice p {
  margin: 4px 0 0;
  font-size: 12px;
  color: #d8c9a6;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(258px, 1fr));
  gap: 18px;
}

.inst-card {
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  overflow: hidden;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.015));
  transition: transform 0.2s ease, border-color 0.2s ease, box-shadow 0.2s ease;
}

.inst-card:hover {
  transform: translateY(-3px);
  border-color: rgba(90, 216, 111, 0.32);
  box-shadow: var(--shadow-soft);
}

.inst-cover {
  position: relative;
  height: 96px;
  padding: 12px 14px;
  background:
    linear-gradient(135deg, rgba(90, 216, 111, 0.22), rgba(124, 92, 255, 0.18)),
    repeating-linear-gradient(45deg, rgba(255, 255, 255, 0.05) 0 8px, transparent 8px 16px);
  overflow: hidden;
}

.cover-glow {
  position: absolute;
  inset: auto -30% -60% -30%;
  height: 140px;
  background: radial-gradient(closest-side, rgba(90, 216, 111, 0.4), transparent);
  filter: blur(6px);
}

.ver {
  position: relative;
  display: inline-block;
  padding: 4px 10px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.4px;
  color: #06120a;
  background: rgba(255, 255, 255, 0.86);
}

.inst-body {
  padding: 14px 16px 16px;
}

.inst-body h3 {
  margin: 0 0 4px;
  font-size: 15px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.path {
  margin: 0 0 14px;
  font-size: 11px;
  color: var(--text-faint);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}

.inst-actions {
  display: flex;
  gap: 8px;
}

.play {
  flex: 1;
}

.del {
  flex: 0 0 auto;
}

.empty-actions {
  display: flex;
  gap: 8px;
  margin-top: 10px;
  flex-wrap: wrap;
  justify-content: center;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 54px 24px;
  text-align: center;
}

.empty-state h2 {
  margin: 14px 0 0;
  font-size: 17px;
}

.empty-state p {
  margin: 0 0 14px;
  font-size: 13px;
  color: var(--text-dim);
}

.empty-art {
  display: grid;
  grid-template-columns: repeat(3, 22px);
  gap: 8px;
  opacity: 0.75;
}

.cube {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  background: linear-gradient(135deg, rgba(90, 216, 111, 0.7), rgba(47, 156, 70, 0.4));
}

.cube.c1,
.cube.c3,
.cube.c7,
.cube.c9 {
  background: rgba(255, 255, 255, 0.07);
}

.modal-mask {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  background: rgba(4, 7, 11, 0.66);
  backdrop-filter: blur(6px);
  z-index: 50;
}

.modal {
  width: 380px;
  padding: 24px;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line-strong);
  background: var(--bg-panel);
  box-shadow: var(--shadow-soft);
}

.modal h2 {
  margin: 0 0 6px;
  font-size: 17px;
}

.modal-sub {
  margin: 0 0 18px;
  font-size: 12px;
  color: var(--text-dim);
  line-height: 1.6;
}

.modal label {
  display: block;
  margin-bottom: 14px;
}

.modal label span {
  display: block;
  margin-bottom: 7px;
  font-size: 12px;
  color: var(--text-dim);
}

.modal input {
  width: 100%;
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 22px;
}
/* ===== MC 启动器主视觉 ===== */
.hero {
  display: grid;
  grid-template-columns: 240px 1fr;
  padding: 0;
  overflow: hidden;
  margin-bottom: 20px;
}

.hero-stage {
  position: relative;
  min-height: 176px;
  display: grid;
  place-items: center;
  background: linear-gradient(180deg, #3a7f42 0%, #23512c 58%, #17241b 100%);
  border-right: 2px solid #0b1015;
}

.hero-stage::after {
  content: "";
  position: absolute;
  inset: 0;
  background-image:
    repeating-linear-gradient(0deg, rgba(0, 0, 0, 0.16) 0 2px, transparent 2px 24px),
    repeating-linear-gradient(90deg, rgba(0, 0, 0, 0.16) 0 2px, transparent 2px 24px);
  opacity: 0.55;
}

.hero-badge {
  position: absolute;
  top: 10px;
  left: 10px;
  z-index: 2;
  padding: 2px 8px;
  border-radius: 3px;
  font-size: 11px;
  font-weight: 800;
  color: #221600;
  background: var(--gold);
  box-shadow: 0 2px 0 rgba(0, 0, 0, 0.45);
}

.hero-blocks {
  position: relative;
  z-index: 1;
  display: grid;
  grid-template-columns: repeat(2, 46px);
  gap: 10px;
}

.blk {
  width: 46px;
  height: 46px;
  border-radius: 3px;
  box-shadow: inset 0 3px 0 rgba(255, 255, 255, 0.25), inset 0 -4px 0 rgba(0, 0, 0, 0.4),
    0 3px 0 rgba(0, 0, 0, 0.45);
}

.blk.grass {
  background: linear-gradient(180deg, #74d155 0 40%, #7d5632 40% 100%);
}

.blk.dirt {
  background: #71512f;
}

.blk.stone {
  background: #7d848b;
}

.blk.diamond {
  background: linear-gradient(180deg, #93f0e7, #37bfb2);
}

.hero-info {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 18px 22px;
}

.hero-info h2 {
  margin: 0;
  font-size: 22px;
  font-weight: 800;
  letter-spacing: 0.4px;
  text-shadow: 0 2px 0 rgba(0, 0, 0, 0.5);
}

.hero-meta {
  margin: 0;
  font-size: 12px;
  color: var(--text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.hero-meta strong {
  color: var(--diamond);
}

.hero-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: auto;
  padding-top: 8px;
}

.play-big {
  font-size: 15px;
  letter-spacing: 1.5px;
  padding: 12px 34px;
}

/* 进阶操作收进折叠里，平时只留启动 */
.adv {
  position: relative;
}

.adv > summary {
  list-style: none;
  cursor: pointer;
  padding: 9px 14px;
  border: 2px solid var(--line, rgba(255, 255, 255, 0.16));
  background: rgba(0, 0, 0, 0.28);
  color: var(--text-dim, #c9c9c9);
  font-size: 12px;
  letter-spacing: 1px;
  user-select: none;
}

.adv > summary::-webkit-details-marker {
  display: none;
}

.adv > summary::after {
  content: "▾";
  margin-left: 6px;
  opacity: 0.7;
}

.adv[open] > summary::after {
  content: "▴";
}

.adv > summary:hover {
  border-color: rgba(98, 204, 85, 0.6);
  color: #eaeaea;
}

.adv-body {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
  padding: 10px;
  border: 2px solid var(--line, rgba(255, 255, 255, 0.14));
  background: rgba(0, 0, 0, 0.32);
}

/* 实例卡片：草方块封面 + 选中/运行态 */
.inst-card {
  cursor: pointer;
  border-width: 2px;
}

.inst-card.active {
  border-color: rgba(98, 204, 85, 0.85);
  box-shadow: 0 0 0 2px rgba(98, 204, 85, 0.18), var(--shadow-soft);
}

.inst-card.active .inst-cover {
  box-shadow: inset 0 0 0 2px rgba(98, 204, 85, 0.5);
}

.inst-cover {
  background:
    linear-gradient(180deg, rgba(116, 209, 85, 0.55) 0 26%, rgba(0, 0, 0, 0.08) 26%),
    repeating-linear-gradient(90deg, rgba(0, 0, 0, 0.16) 0 2px, transparent 2px 22px),
    linear-gradient(160deg, #3f7a45 0%, #2a4b31 55%, #1d2a20 100%);
}

.inst-card.running .inst-cover::after {
  content: "运行中";
  position: absolute;
  right: 10px;
  bottom: 10px;
  padding: 2px 8px;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 800;
  color: #06210a;
  background: var(--grass-hi);
  box-shadow: 0 2px 0 rgba(0, 0, 0, 0.4);
}

.ver {
  color: #1b1200;
  background: var(--gold);
  border-radius: 3px;
  box-shadow: 0 2px 0 rgba(0, 0, 0, 0.45);
}
/* 首页：让主视觉吃掉剩余高度，避免下方大片死白 */
.home-page {
  display: flex;
  flex-direction: column;
}

.home-page .hero {
  flex: 1;
  min-height: 240px;
}

.home-page .grid {
  flex: 1;
  align-content: start;
}

.home-page .empty-state {
  flex: 1;
}

.pick-list {
  list-style: none;
  margin: 14px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.pick-row {
  display: grid;
  grid-template-columns: 10px 1fr auto auto 14px;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 15px 18px;
  cursor: pointer;
  text-align: left;
  color: var(--text);
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  background: var(--bg-panel, #17161f);
  transition: border-color 0.16s ease, transform 0.16s ease;
}

.pick-row:hover {
  border-color: #6c8cff;
  transform: translateX(2px);
}

.pick-name {
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.pick-ver {
  font-size: 11px;
  padding: 3px 9px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.07);
  color: var(--text-dim);
}

.pick-state {
  font-size: 11px;
  color: #4ad7c8;
}

.pick-arrow {
  font-size: 16px;
  color: var(--text-dim);
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.18);
}

.dot.running {
  background: #4ad7c8;
  box-shadow: 0 0 8px #4ad7c8;
}

.drop-hint {
  margin: 16px 2px 0;
  font-size: 11.5px;
  color: var(--text-dim);
}

.detail {
  display: flex;
  justify-content: space-between;
  gap: 24px;
  margin-top: 14px;
  padding: 22px 24px;
}

.detail-main {
  min-width: 0;
}

.detail-main h2 {
  margin: 10px 0 8px;
  font-size: 22px;
}

.detail-meta {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.chip {
  font-size: 11px;
  padding: 3px 9px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.07);
  color: var(--text-dim);
}

.chip.muted {
  opacity: 0.7;
}

.path-box {
  margin: 0 0 14px;
  font-size: 11.5px;
  color: var(--text-dim);
}

.path-box summary {
  cursor: pointer;
}

.path-box code {
  display: block;
  margin-top: 6px;
  padding: 8px 10px;
  border-radius: 8px;
  background: rgba(0, 0, 0, 0.28);
  word-break: break-all;
}

.detail-side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
}

.danger-hint {
  margin: 0;
  max-width: 200px;
  font-size: 11.5px;
  text-align: right;
  color: #ff9c9c;
}

</style>
