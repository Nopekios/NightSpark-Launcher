<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import CardSpotlight from "@/components/ui/card-spotlight/CardSpotlight.vue";
import NumberTicker from "@/components/ui/number-ticker/NumberTicker.vue";
import BlurReveal from "@/components/ui/blur-reveal/BlurReveal.vue";
import GlowBorder from "@/components/ui/glow-border/GlowBorder.vue";
import InteractiveHoverButton from "@/components/ui/interactive-hover-button/InteractiveHoverButton.vue";
import ParticlesBg from "@/components/ui/particles-bg/ParticlesBg.vue";
import InstanceBgPanel from "@/components/ui/instance-bg/InstanceBgPanel.vue";
import type { GameInstance } from "../api";

const { t } = useI18n();

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

/* ── 实例卡片背景：纯前端配置，图片压成 dataURL 跟其它参数一起塞 localStorage ── */
type TileBgType = "none" | "image" | "gradient";

interface TileBg {
  type: TileBgType;
  image: string;
  color: string;
  opacity: number;
  blur: number;
  scrim: number;
  position: string;
}

const BG_STORE_KEY = "ll.instance.bg.v1";
const BG_GRADIENT_DEFAULT = "linear-gradient(135deg, #0d2438 0%, #0a1524 60%, #12324d 100%)";

function defaultBg(): TileBg {
  return {
    type: "none",
    image: "",
    color: BG_GRADIENT_DEFAULT,
    opacity: 85,
    blur: 0,
    scrim: 34,
    position: "50% 50%",
  };
}

function clampNum(value: unknown, min: number, max: number, fallback: number) {
  const num = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(num)) return fallback;
  return Math.min(max, Math.max(min, num));
}

// 存进来的可能是旧版本或被手改过的数据，统一洗一遍再上屏
function normalizeBg(raw: unknown): TileBg {
  const base = defaultBg();
  if (!raw || typeof raw !== "object") return base;
  const src = raw as Record<string, unknown>;
  const type: TileBgType = src.type === "image" || src.type === "gradient" ? src.type : "none";
  return {
    type,
    image: typeof src.image === "string" ? src.image : base.image,
    color: typeof src.color === "string" && src.color.trim() ? src.color : base.color,
    opacity: clampNum(src.opacity, 0, 100, base.opacity),
    blur: clampNum(src.blur, 0, 30, base.blur),
    scrim: clampNum(src.scrim, 0, 90, base.scrim),
    position: typeof src.position === "string" && src.position.trim() ? src.position : base.position,
  };
}

function readBgStore(): Record<string, TileBg> {
  const out: Record<string, TileBg> = {};
  try {
    const raw = localStorage.getItem(BG_STORE_KEY);
    if (!raw) return out;
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return out;
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      const cfg = normalizeBg(value);
      if (cfg.type !== "none") out[id] = cfg;
    }
  } catch (err) {
    console.warn("[instance-bg] 读取本地背景配置失败", err);
  }
  return out;
}

const bgMap = ref<Record<string, TileBg>>(readBgStore());
const bgSaveError = ref("");
let bgSaveTimer: number | undefined;

function persistBg() {
  // 「无背景」不落盘，避免本地存储里堆一堆空配置
  const data: Record<string, TileBg> = {};
  for (const [id, cfg] of Object.entries(bgMap.value)) {
    if (cfg.type === "none") continue;
    data[id] = cfg;
  }
  try {
    localStorage.setItem(BG_STORE_KEY, JSON.stringify(data));
    bgSaveError.value = "";
  } catch (err) {
    // 基本都是撞了 localStorage 5MB 上限，说清楚别让用户以为已经存住了
    bgSaveError.value = t("home.bgStoreFull");
    console.warn("[instance-bg] 写入本地存储失败", err);
  }
}

// 拖滑块时会连续改动，防抖一下再写，省得每帧都序列化一遍 base64
watch(
  bgMap,
  () => {
    if (bgSaveTimer) window.clearTimeout(bgSaveTimer);
    bgSaveTimer = window.setTimeout(persistBg, 260);
  },
  { deep: true }
);

onBeforeUnmount(() => {
  if (bgSaveTimer) window.clearTimeout(bgSaveTimer);
  persistBg();
});

const bgEditor = ref<{ id: string; x: number; y: number } | null>(null);

function bgOf(id: string): TileBg {
  return bgMap.value[id] ?? defaultBg();
}

const editorConfig = computed(() => (bgEditor.value ? bgOf(bgEditor.value.id) : defaultBg()));

const bgEditorName = computed(() => {
  const id = bgEditor.value?.id;
  if (!id) return "";
  return props.instances.find((item) => item.id === id)?.name ?? "";
});

function hasBg(id: string) {
  const cfg = bgMap.value[id];
  if (!cfg) return false;
  // 选了「图片」但还没挑图时不算有背景，别把卡片压成一块死黑
  if (cfg.type === "image") return !!cfg.image;
  return cfg.type === "gradient";
}

function toggleBgEditor(e: MouseEvent, item: GameInstance) {
  // 卡片本身点了是进实例，这里必须掐掉冒泡
  e.stopPropagation();
  if (bgEditor.value?.id === item.id) {
    bgEditor.value = null;
    return;
  }
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  bgEditor.value = { id: item.id, x: rect.left, y: rect.bottom + 10 };
}

function updateBg(patch: Partial<TileBg>) {
  const id = bgEditor.value?.id;
  if (!id) return;
  bgMap.value = { ...bgMap.value, [id]: normalizeBg({ ...bgOf(id), ...patch }) };
}

function resetBg() {
  const id = bgEditor.value?.id;
  if (!id) return;
  const next = { ...bgMap.value };
  delete next[id];
  bgMap.value = next;
}

function closeBgEditor() {
  bgEditor.value = null;
}

// 背景层的实际样式：opacity/blur/object-position 都落到内联样式上，改一个参数立刻可见
function bgLayerStyle(id: string): Record<string, string> {
  const cfg = bgOf(id);
  if (cfg.type === "none") return {};
  const style: Record<string, string> = { opacity: (cfg.opacity / 100).toFixed(2) };
  if (cfg.blur > 0) {
    style.filter = `blur(${cfg.blur}px)`;
    // 模糊会把图层边缘一起糊透，稍微放大把虚边推到磁贴裁切之外
    style.transform = `scale(${(1 + cfg.blur / 60).toFixed(3)})`;
  }
  if (cfg.type === "image") {
    if (cfg.image) style.backgroundImage = `url(${cfg.image})`;
    style.backgroundSize = "cover";
    style.backgroundPosition = cfg.position;
  } else {
    style.background = cfg.color;
  }
  return style;
}

function scrimStyle(id: string): Record<string, string> {
  const cfg = bgOf(id);
  if (cfg.type === "none" || cfg.scrim <= 0) return {};
  return { opacity: (cfg.scrim / 100).toFixed(2) };
}

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

// inspira 3d-card / glare-card 思路：磁贴跟随光标做轻量 3D 倾斜，并同步反光坐标。
function handleTilt(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement | null;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const px = (e.clientX - r.left) / r.width - 0.5;
  const py = (e.clientY - r.top) / r.height - 0.5;
  el.style.setProperty("--ry", `${(px * 18).toFixed(2)}deg`);
  el.style.setProperty("--rx", `${(-py * 13).toFixed(2)}deg`);
  el.style.setProperty("--mx", `${((px + 0.5) * 100).toFixed(1)}%`);
  el.style.setProperty("--my", `${((py + 0.5) * 100).toFixed(1)}%`);
}

function resetTilt(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement | null;
  if (!el) return;
  el.style.setProperty("--ry", "0deg");
  el.style.setProperty("--rx", "0deg");
}

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
    <ParticlesBg class="page-particles" color="#3ea6ff" :quantity="90" :staticity="50" :ease="45" />
    <header class="page-head">
      <div>
        <h1>
          <BlurReveal :delay="0.06" :duration="0.6">
            <span>{{ hero ? t("home.detailTitle") : t("home.listTitle") }}</span>
          </BlurReveal>
        </h1>
        <p v-if="hero">{{ t("home.versionLabel", { version: hero.version }) }}</p>
        <p v-else>
          {{ t("home.countPrefix") }}
          <NumberTicker
            class="count-ticker"
            :value="instances.length"
            :duration="900"
            :decimal-places="0"
          />
          {{ t("home.countSuffix") }}<span v-if="instances.length > 0">{{ t("home.countTail") }}</span>
        </p>
      </div>
      <div class="head-actions">
        <button v-if="hero" @click="backToList">{{ t("home.backToList") }}</button>
        <template v-else>
          <input v-if="instances.length > 5" v-model="query" class="search" :placeholder="t('home.searchPlaceholder')" />
          <button @click="openCreate">{{ t("home.create") }}</button>
        </template>
      </div>
    </header>

    <div class="free-banner">
      <span class="free-dot" aria-hidden="true"></span>
      <div class="free-text">
        <strong>{{ t("home.freeNotice") }}</strong>
        <span>{{ t("home.freeNoticeNote") }}</span>
      </div>
    </div>

    <template v-if="hero">
      <div class="detail panel">
        <div class="detail-main">
          <span class="tag" :class="{ muted: !hasAccount }">
            {{ heroPid ? t("home.runningWithPid", { pid: heroPid }) : hasAccount ? t("home.ready") : t("home.needAccount") }}
          </span>
          <h2 :title="hero.name">{{ hero.name }}</h2>
          <div class="detail-meta">
            <span class="chip">{{ hero.version }}</span>
            <span class="chip muted">{{ t("home.isolatedDir") }}</span>
          </div>
          <details class="path-box">
            <summary>{{ t("home.gameDir") }}</summary>
            <code>{{ hero.game_dir }}</code>
          </details>
          <div class="hero-actions">
            <button v-if="!hasAccount" class="primary play-big" @click="emit('open-account')">{{ t("home.loginMs") }}</button>
            <button
              v-else
              class="primary play-big"
              :disabled="busy || launchingId === hero.id"
              :title="t('home.launchTitle')"
              @click="emit('launch', hero)"
            >
              {{ launchingId === hero.id ? t("home.launching") : heroPid ? t("home.launchAgain") : t("home.launch") }}
            </button>
            <button v-if="heroPid" class="ghost-danger" @click="emit('kill', hero)">{{ t("home.kill") }}</button>
            <details class="adv">
              <summary>{{ t("home.more") }}</summary>
              <div class="adv-body">
                <button :disabled="busy" :title="t('home.fetchModsTitle')"
                  @click="emit('fetch-mods', hero)">
                  {{ t("home.fetchMods") }}
                </button>
                <input
                  v-model="loaderSpec"
                  :disabled="busy"
                  :placeholder="t('home.loaderPlaceholder')"
                  :title="t('home.loaderTitle')"
                />
                <button :disabled="busy || loaderSpec.trim().length === 0"
                  :title="t('home.installLoaderTitle')"
                  @click="emit('install-loader', { item: hero, spec: loaderSpec.trim() })">
                  {{ t("home.installLoader") }}
                </button>
                <button :disabled="busy"
                  :title="t('home.syncTitle')"
                  @click="emit('sync-files', hero)">
                  {{ t("home.syncFiles") }}
                </button>
                <button :disabled="busy" :title="t('home.verifyTitle')"
                  @click="emit('verify', hero)">
                  {{ t("home.verify") }}
                </button>
              </div>
            </details>
          </div>
        </div>
        <div class="detail-side">
          <template v-if="pendingDelete === hero.id">
            <p class="danger-hint">{{ t("home.deleteWarn") }}</p>
            <button class="ghost-danger" :disabled="busy" @click="emit('remove', hero)">{{ t("home.confirmDelete") }}</button>
            <button :disabled="busy" @click="pendingDelete = null">{{ t("common.cancel") }}</button>
          </template>
          <button v-else class="ghost-danger del" :disabled="busy" @click="pendingDelete = hero.id">
            {{ t("home.deleteInstance") }}
          </button>
        </div>
      </div>
    </template>

    <template v-else>
      <div v-if="instances.length === 0" class="hero panel">
        <div class="hero-stage">
          <span class="hero-badge">{{ t("home.notCreated") }}</span>
          <div class="orb" aria-hidden="true">
            <span class="orb-ring r1" />
            <span class="orb-ring r2" />
            <span class="orb-core" />
          </div>
        </div>
        <div class="hero-info">
          <span class="tag muted">{{ hasAccount ? t("home.stepOne") : t("home.needAccount") }}</span>
          <h2>{{ hasAccount ? t("home.emptyTitle") : t("home.needAccountTitle") }}</h2>
          <p class="hero-meta">
            {{
              hasAccount
                ? t("home.emptyHint")
                : t("home.needAccountHint")
            }}
          </p>
          <div class="hero-actions">
            <button v-if="!hasAccount" class="primary play-big" @click="emit('open-account')">{{ t("home.loginMs") }}</button>
            <button v-else class="primary play-big" @click="emit('import-local')">{{ t("home.importLocal") }}</button>
            <button v-if="hasAccount" @click="openCreate">{{ t("home.create") }}</button>
            <button v-else @click="emit('import-local')">{{ t("home.importLocal") }}</button>
          </div>
        </div>
      </div>

      <div v-else-if="filtered.length === 0" class="empty-state panel">
        <h2>{{ t("home.noMatchTitle") }}</h2>
        <p>{{ t("home.noMatchHint") }}</p>
      </div>

      <ul v-else class="tile-grid">
        <li
          v-for="(item, index) in filtered"
          :key="item.id"
          class="tile anim-in"
          :style="`--d:${index * 45}ms`"
          @click="enter(item)"
          @mousemove="handleTilt"
          @mouseleave="resetTilt"
        >
          <span
            v-if="hasBg(item.id)"
            class="tile-bg"
            :style="bgLayerStyle(item.id)"
            aria-hidden="true"
          />
          <span
            v-if="hasBg(item.id) && bgOf(item.id).scrim > 0"
            class="tile-scrim"
            :style="scrimStyle(item.id)"
            aria-hidden="true"
          />
          <CardSpotlight
            class="tile-spotlight"
            slot-class="tile-spotlight-inner"
            gradient-color="rgba(0, 149, 255, 0.34)"
            :gradient-size="300"
            :gradient-opacity="0.9"
          >
            <div class="tile-body">
              <span class="tile-ver">{{ item.version }}</span>
              <span class="tile-top">
                <span class="dot" :class="{ running: !!runningPids[item.id] }" />
                <span class="tile-flag">{{ runningPids[item.id] ? t("home.running") : t("home.readyTag") }}</span>
              </span>
              <span class="tile-name" :title="item.name">{{ item.name }}</span>
              <span class="tile-foot">
                <InteractiveHoverButton
                  class="tile-cta"
                  :text="runningPids[item.id] ? t('home.running') : t('home.enter')"
                  @click.stop="enter(item)"
                />
              </span>
            </div>
          </CardSpotlight>
          <span class="tile-glare" aria-hidden="true" />
          <button
            type="button"
            class="tile-bg-btn"
            :class="{ 'is-on': hasBg(item.id), 'is-active': bgEditor?.id === item.id }"
            :title="hasBg(item.id) ? t('home.adjustBg') : t('home.setBg')"
            @click.stop="toggleBgEditor($event, item)"
          >
            <svg
              viewBox="0 0 24 24"
              width="13"
              height="13"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <rect x="3.4" y="4.2" width="17.2" height="15.6" rx="3" />
              <circle cx="8.6" cy="9.2" r="1.5" />
              <path d="M4.2 16.6l4.3-4 3.3 2.9 2.9-2.5 4.9 4.4" />
            </svg>
          </button>
        </li>
        <li
          class="tile tile-new"
          @mousemove="handleTilt"
          @mouseleave="resetTilt"
        >
          <button class="tile-body" @click="openCreate">
            <GlowBorder
              class="tile-glow"
              color="#0095ff"
              :border-radius="18"
              :border-width="1.5"
              :duration="7"
            />
            <span class="tile-plus">+</span>
            <span class="tile-name">{{ t("home.create") }}</span>
            <span class="tile-foot"><span class="tile-enter">{{ t("home.createHint") }}</span></span>
          </button>
        </li>
      </ul>
      <InstanceBgPanel
        v-if="bgEditor"
        :config="editorConfig"
        :anchor="{ x: bgEditor.x, y: bgEditor.y }"
        :name="bgEditorName"
        :hint="bgSaveError"
        @update="updateBg"
        @reset="resetBg"
        @close="closeBgEditor"
      />
      <p v-if="instances.length > 0" class="drop-hint">
        {{ t("home.dragHint") }}
      </p>
    </template>

    <Transition name="toast">
      <div v-if="showCreate" class="modal-mask" @click.self="showCreate = false">
        <div class="modal">
          <h2>{{ t("home.create") }}</h2>
          <p class="modal-sub">{{ t("home.createSub") }}</p>
          <label>
            <span>{{ t("home.nameLabel") }}</span>
            <input v-model="name" autofocus :placeholder="t('home.namePlaceholder')" @keyup.enter="submit" />
          </label>
          <label>
            <span>{{ t("home.versionField") }}</span>
            <input v-model="version" :placeholder="t('home.versionPlaceholder')" @keyup.enter="submit" />
          </label>
          <div class="modal-actions">
            <button @click="showCreate = false">{{ t("common.cancel") }}</button>
            <button class="primary" :disabled="busy || !name.trim() || !version.trim()" @click="submit">{{ t("home.confirmCreate") }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </section>
</template>

<style scoped>
.home-page {
  position: relative;
  isolation: isolate;
  display: flex;
  flex-direction: column;
  gap: 22px;
}

.home-page > :deep(.page-particles) {
  position: absolute;
  inset: -28px -26px auto -26px;
  height: 460px;
  z-index: 0;
  opacity: 0.62;
  pointer-events: none;
  mask-image: radial-gradient(ellipse 120% 96% at 50% 0%, #000 0%, rgba(0, 0, 0, 0.5) 52%, transparent 84%);
  -webkit-mask-image: radial-gradient(
    ellipse 120% 96% at 50% 0%,
    #000 0%,
    rgba(0, 0, 0, 0.5) 52%,
    transparent 84%
  );
}

.home-page .page-head,
.home-page .tile-grid,
.home-page .hero,
.home-page .empty-state {
  position: relative;
  z-index: 1;
}

.tile {
  cursor: pointer;
}

.tile-cta {
  flex: 1 1 100%;
  width: 100% !important;
  min-height: 30px;
  /* 通栏按钮：左右各留 14px，箭头就是磁贴右侧最近的那个元素 */
  padding: 4px 14px !important;
  border-radius: 0 0 17px 17px !important;
  border-color: rgba(126, 160, 199, 0.18) !important;
  border-right: 0 !important;
  border-bottom: 0 !important;
  border-left: 0 !important;
  background: rgba(255, 255, 255, 0.016) !important;
  font-size: 11px;
  line-height: 1.5;
  font-weight: 600;
  letter-spacing: 0.01em;
  color: var(--text-dim);
  transition: border-color 0.25s ease, color 0.25s ease, background 0.25s ease;
}

/* 按钮内部文字左、箭头贴最右；hover 蓝底由组件内的填充层铺满并对齐边界 */

.tile:hover .tile-cta {
  border-color: rgba(0, 149, 255, 0.55) !important;
  color: #eaf3ff;
}

/* ── 顶部：标题 + 操作 ── */
.head-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

.search {
  width: 190px;
  padding: 7px 14px;
  border-radius: 999px;
  background: rgba(10, 18, 28, 0.9);
}

.run-state {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 8px;
  font-size: 11px;
  color: var(--ice);
}

.link {
  padding: 0;
  border: none;
  background: none;
  box-shadow: none;
  color: var(--text-dim);
  font-size: 11px;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.link:hover {
  color: var(--text);
}

.link.danger {
  color: #ff9daa;
}

.link.danger:hover {
  color: var(--danger);
}

/* ── 空态主视觉 ── */
.hero {
  display: grid;
  grid-template-columns: 236px 1fr;
  align-items: center;
  gap: 30px;
  padding: 34px 36px;
  overflow: hidden;
}

.hero::after {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: radial-gradient(560px 260px at 8% 0%, rgba(0, 149, 255, 0.12), transparent 70%);
}

.hero-stage {
  position: relative;
  display: grid;
  place-items: center;
  width: 100%;
  aspect-ratio: 1;
}

.hero-badge {
  position: absolute;
  top: 0;
  left: 0;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-faint);
  border: 1px solid rgba(126, 160, 199, 0.2);
  background: rgba(8, 13, 20, 0.7);
}

/* 抽象光球：替代原来的像素方块 */
.orb {
  position: relative;
  display: grid;
  place-items: center;
  width: 168px;
  height: 168px;
}

.orb-ring {
  position: absolute;
  border-radius: 999px;
  border: 1px solid rgba(0, 149, 255, 0.28);
}

.orb-ring.r1 {
  inset: 0;
  border-style: dashed;
  opacity: 0.6;
  animation: ll-beam-spin 26s linear infinite;
}

.orb-ring.r2 {
  inset: 24px;
  border-color: rgba(53, 224, 200, 0.24);
  animation: ll-beam-spin 18s linear infinite reverse;
}

.orb-core {
  width: 74px;
  height: 74px;
  border-radius: 999px;
  background: radial-gradient(circle at 34% 28%, #b7e4ff 0%, #3eb1ff 34%, #0072c6 68%, #063f6d 100%);
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.18), 0 0 42px rgba(0, 149, 255, 0.62),
    inset 0 -8px 18px rgba(0, 20, 40, 0.6);
  animation: ll-float 6.5s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}

.hero-info {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 12px;
  z-index: 1;
}

.hero-info h2 {
  margin: 0;
  font-size: 25px;
  font-weight: 700;
  letter-spacing: -0.025em;
  line-height: 1.18;
}

.hero-meta {
  margin: 0;
  max-width: 52ch;
  font-size: 12.5px;
  line-height: 1.72;
  color: var(--text-dim);
}

.hero-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  margin-top: 6px;
}

/* 主按钮：渐变 + 扫光 */
.play-big {
  position: relative;
  overflow: hidden;
  padding: 11px 26px;
  font-size: 13.5px;
  font-weight: 700;
  border-radius: 12px;
}

.play-big::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 42%;
  background: linear-gradient(100deg, transparent, rgba(255, 255, 255, 0.34), transparent);
  transform: translateX(-160%);
  transition: none;
}

.play-big:hover:not(:disabled)::after {
  animation: ll-shine 0.9s cubic-bezier(0.22, 1, 0.36, 1) 1;
}

/* ── 实例详情 ── */
.detail {
  display: grid;
  grid-template-columns: 1fr 200px;
  gap: 26px;
  padding: 26px 28px;
}

.detail-main {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.detail-main h2 {
  margin: 0;
  font-size: 27px;
  font-weight: 700;
  letter-spacing: -0.028em;
  line-height: 1.18;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.detail-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.chip {
  display: inline-flex;
  align-items: center;
  padding: 3px 11px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 650;
  letter-spacing: 0.01em;
  color: #bfe0ff;
  background: rgba(0, 149, 255, 0.11);
  border: 1px solid rgba(0, 149, 255, 0.22);
}

.chip.muted {
  color: var(--text-dim);
  background: rgba(126, 160, 199, 0.07);
  border-color: rgba(126, 160, 199, 0.16);
}

.path-box {
  border: 1px solid rgba(126, 160, 199, 0.12);
  border-radius: 10px;
  background: rgba(0, 0, 0, 0.24);
  overflow: hidden;
}

.path-box summary {
  padding: 8px 13px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-dim);
  cursor: pointer;
  list-style: none;
  transition: color 0.2s ease, background 0.2s ease;
}

.path-box summary::-webkit-details-marker {
  display: none;
}

.path-box summary:hover {
  color: var(--text);
  background: rgba(126, 160, 199, 0.05);
}

.path-box code {
  display: block;
  padding: 0 13px 10px;
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
  color: #7f95ab;
  word-break: break-all;
}

.adv {
  position: relative;
}

.adv summary {
  padding: 8px 16px;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-dim);
  border: 1px solid var(--line);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.03);
  cursor: pointer;
  list-style: none;
  transition: color 0.2s ease, border-color 0.2s ease, background 0.2s ease;
}

.adv summary::-webkit-details-marker {
  display: none;
}

.adv summary:hover {
  color: var(--text);
  border-color: rgba(0, 149, 255, 0.34);
  background: rgba(0, 149, 255, 0.07);
}

.adv[open] summary {
  color: var(--accent-hi);
  border-color: rgba(0, 149, 255, 0.4);
  background: rgba(0, 149, 255, 0.1);
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.adv-body {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 12px;
  border: 1px solid rgba(0, 149, 255, 0.24);
  border-top: none;
  border-radius: 0 0 10px 10px;
  background: rgba(6, 11, 18, 0.6);
  animation: ll-fade-up 0.24s cubic-bezier(0.22, 1, 0.36, 1) both;
}

.adv-body input {
  width: 190px;
  padding: 7px 12px;
  font-size: 12px;
}

.detail-side {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  justify-content: flex-start;
  gap: 8px;
  padding-left: 22px;
  border-left: 1px solid rgba(126, 160, 199, 0.1);
}

.danger-hint {
  margin: 0 0 2px;
  font-size: 11.5px;
  line-height: 1.6;
  color: #ff9daa;
}

.del {
  margin-top: auto;
}

/* ── 实例列表 ── */
.pick-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
}

.pick-row {
  position: relative;
  display: grid;
  grid-template-columns: 10px 1fr auto 76px 14px;
  align-items: center;
  gap: 14px;
  width: 100%;
  padding: 15px 18px;
  border-radius: 13px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.035) 0%, rgba(255, 255, 255, 0.008) 100%),
    rgba(12, 18, 27, 0.82);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  text-align: left;
  transition: border-color 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.22s cubic-bezier(0.22, 1, 0.36, 1), transform 0.22s cubic-bezier(0.22, 1, 0.36, 1),
    background 0.22s ease;
}

.pick-row::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  width: 2px;
  height: 0;
  border-radius: 999px;
  background: linear-gradient(180deg, #3eb1ff, #0072c6);
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.85);
  transform: translateY(-50%);
  transition: height 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}

.pick-row:hover {
  border-color: rgba(0, 149, 255, 0.32);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.09) 0%, rgba(0, 149, 255, 0.02) 100%),
    rgba(12, 18, 27, 0.9);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 16px 34px -20px rgba(0, 149, 255, 0.8);
  transform: translateX(2px);
}

.pick-row:hover::before {
  height: 22px;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: rgba(126, 160, 199, 0.32);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1);
  transition: background 0.2s ease, box-shadow 0.2s ease;
}

.dot.running {
  background: #2fd07f;
  box-shadow: 0 0 0 3px rgba(47, 208, 127, 0.14), 0 0 12px rgba(47, 208, 127, 0.9);
  animation: ll-glow-pulse 2s ease-in-out infinite;
}

.pick-name {
  font-size: 13.5px;
  font-weight: 650;
  letter-spacing: -0.005em;
  color: var(--text);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.pick-ver {
  font-size: 11.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
  letter-spacing: 0.01em;
  padding: 3px 9px;
  border-radius: 999px;
  background: rgba(126, 160, 199, 0.08);
}

.pick-state {
  font-size: 11px;
  font-weight: 650;
  color: #2fd07f;
  text-align: right;
}

.pick-arrow {
  font-size: 17px;
  line-height: 1;
  color: var(--text-faint);
  transition: transform 0.24s cubic-bezier(0.22, 1, 0.36, 1), color 0.2s ease;
}

.pick-row:hover .pick-arrow {
  color: var(--accent-hi);
  transform: translateX(3px);
}

.drop-hint {
  margin: 2px 0 0;
  font-size: 11.5px;
  color: var(--text-faint);
  letter-spacing: 0.005em;
}

.empty-state {
  padding: 44px;
  text-align: center;
}

.empty-state h2 {
  margin: 0 0 6px;
  font-size: 15px;
  font-weight: 650;
}

.empty-state p {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-dim);
}

/* ── 新建实例弹层 ── */
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

.modal {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: min(420px, 92vw);
  padding: 24px 26px 22px;
  border-radius: 18px;
  border: 1px solid rgba(126, 160, 199, 0.18);
  background: linear-gradient(180deg, #0f1826 0%, #0a111b 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 40px 90px -30px rgba(0, 0, 0, 1);
}

.modal h2 {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
  letter-spacing: -0.02em;
}

.modal-sub {
  margin: -6px 0 2px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-dim);
}

.modal label {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.modal label span {
  font-size: 11px;
  font-weight: 650;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 4px;
}
/* ── 自创磁贴阵列（替掉 MC 官方那种单列列表） ── */
.tile-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(236px, 1fr));
  gap: 14px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.tile {
  position: relative;
  min-height: 178px;
  border-radius: 18px;
  overflow: hidden;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: linear-gradient(158deg, rgba(22, 35, 50, 0.9) 0%, rgba(9, 14, 22, 0.93) 64%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05), 0 26px 54px -44px rgba(0, 0, 0, 0.95);
  /* 3D 倾斜挂在磁贴本体上；入场动画不能带 transform，否则会盖掉倾斜变量 */
  transform: perspective(900px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)) translateY(0);
  transform-style: preserve-3d;
  will-change: transform;
  transition: transform 0.32s cubic-bezier(0.22, 1, 0.36, 1), border-color 0.3s ease,
    box-shadow 0.3s ease;
}

/* 磁贴入场改成纯透明度+模糊，避免 transform 动画与倾斜互斥 */
.tile.anim-in {
  animation: ll-tile-enter var(--dur-slow) var(--ease) both;
}

@keyframes ll-tile-enter {
  from {
    opacity: 0;
    filter: blur(7px);
  }
  to {
    opacity: 1;
    filter: blur(0);
  }
}

/* 右下角色调：补一层同主题蓝辉，避免右下角发死黑、与整体脱节 */
.tile::before {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 2;
  border-radius: inherit;
  pointer-events: none;
  background: radial-gradient(120% 96% at 100% 100%, rgba(0, 149, 255, 0.2), rgba(0, 149, 255, 0) 58%);
  opacity: 0.85;
  transition: opacity 0.3s ease;
}

.tile:hover::before {
  opacity: 1;
}

.tile::after {
  content: "";
  position: absolute;
  left: 16%;
  right: 16%;
  top: 0;
  z-index: 5;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(0, 149, 255, 0.95), transparent);
  opacity: 0;
  transition: opacity 0.3s ease;
}

.tile:hover {
  transform: perspective(900px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg))
    translateY(-3px);
  border-color: rgba(126, 160, 199, 0.32);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08), 0 34px 60px -40px rgba(0, 0, 0, 0.95);
}

/* inspira glare-card 思路：白色反光 + 弱虹彩，soft-light 混合，只在悬停时淡入 */
.tile-glare {
  position: absolute;
  inset: 0;
  z-index: 4;
  border-radius: inherit;
  pointer-events: none;
  opacity: 0;
  mix-blend-mode: soft-light;
  transition: opacity 0.35s ease;
  background:
    radial-gradient(
      240px circle at var(--mx, 50%) var(--my, 50%),
      rgba(255, 255, 255, 0.5),
      rgba(255, 255, 255, 0) 64%
    ),
    linear-gradient(
      118deg,
      rgba(255, 120, 190, 0.22) 0%,
      rgba(90, 200, 255, 0.22) 38%,
      rgba(255, 220, 130, 0.2) 66%,
      rgba(140, 255, 220, 0.2) 100%
    );
}

.tile:hover .tile-glare {
  opacity: 1;
}

.tile-body::before {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
  pointer-events: none;
  opacity: 0;
  background: linear-gradient(115deg, transparent 34%, rgba(255, 255, 255, 0.07) 48%, transparent 62%);
  background-size: 240% 100%;
  transition: opacity 0.3s ease;
}

.tile:hover .tile-body::before {
  opacity: 1;
  animation: ll-tile-sweep 1.5s ease-out 1;
}

@keyframes ll-tile-sweep {
  0% {
    background-position: 140% 0;
  }
  100% {
    background-position: -60% 0;
  }
}

.tile:hover::after {
  opacity: 1;
}

.tile-body {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 0;
  width: 100%;
  height: 100%;
  min-height: 178px;
  padding: 16px 18px 14px;
  border: none;
  border-radius: inherit;
  background: transparent;
  box-shadow: none;
  text-align: left;
  cursor: pointer;
  color: inherit;
}

.tile-body:hover {
  transform: none;
  box-shadow: none;
  border-color: transparent;
  background: transparent;
}

/* inspira CardSpotlight：光斑跟随光标由组件内的 motion 值驱动 */
.tile-spotlight {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: inherit;
  overflow: hidden;
  border: 0;
  border-radius: inherit;
  background: transparent;
  color: inherit;
}

.tile-spotlight :deep(.tile-spotlight-inner) {
  display: flex;
  width: 100%;
  height: 100%;
}

/* 页头实例数：NumberTicker 默认是黑字，跟着主题走 */
.count-ticker {
  color: var(--grass-hi);
  font-weight: 700;
}

.tile-ver {
  position: absolute;
  right: 12px;
  top: 2px;
  font-size: 40px;
  font-weight: 800;
  letter-spacing: -0.06em;
  color: #cfe8ff;
  opacity: 0.07;
  pointer-events: none;
  font-variant-numeric: tabular-nums;
}

.tile-top {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 10px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.tile-name {
  margin-top: 16px;
  font-size: 16px;
  font-weight: 700;
  letter-spacing: -0.02em;
  line-height: 1.32;
  color: var(--text);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.tile-foot {
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  /* 通栏到底边：margin-top:auto 把操作条推到底，左右到底让箭头贴住磁贴最右侧 */
  margin: auto -18px -14px;
  padding: 13px 0 0;
  font-size: 11px;
  color: var(--text-dim);
}

.tile-arrow {
  transition: transform 0.28s cubic-bezier(0.22, 1, 0.36, 1), color 0.28s ease;
}

.tile:hover .tile-arrow {
  transform: translateX(3px);
  color: var(--accent);
}

.tile-new {
  border-style: dashed;
  border-color: rgba(126, 160, 199, 0.2);
  background: rgba(12, 18, 27, 0.5);
}

.tile-new:hover {
  border-color: rgba(0, 149, 255, 0.42);
  box-shadow: 0 40px 70px -46px rgba(0, 149, 255, 0.6);
}

.tile-new .tile-body {
  justify-content: center;
  align-items: center;
  text-align: center;
  gap: 10px;
}

.tile-plus {
  display: grid;
  place-items: center;
  width: 40px;
  height: 40px;
  border-radius: 999px;
  font-size: 22px;
  font-weight: 300;
  line-height: 1;
  color: #9fc6e8;
  border: 1px dashed rgba(126, 160, 199, 0.4);
  transition: color 0.3s ease, border-color 0.3s ease, box-shadow 0.3s ease, transform 0.3s ease;
}

.tile-new:hover .tile-plus {
  color: #dff0ff;
  border-color: rgba(0, 149, 255, 0.7);
  box-shadow: 0 0 22px rgba(0, 149, 255, 0.5);
  transform: rotate(90deg);
}

.tile-new .tile-name {
  margin-top: 0;
  font-size: 13.5px;
}

.tile-new .tile-foot {
  border-top: none;
  justify-content: center;
}

/* ── 实例卡片自定义背景：底图层 + 可读性遮罩 + 悬浮入口 ── */
.tile-bg {
  position: absolute;
  inset: 0;
  z-index: 0;
  display: block;
  border-radius: inherit;
  pointer-events: none;
  background-color: #0a1220;
  background-repeat: no-repeat;
  background-position: 50% 50%;
  background-size: cover;
  transform-origin: center;
  transition: opacity 0.25s ease, filter 0.25s ease;
}

/* 暗化遮罩压在底图之上、内容之下，保证实例名和按钮文字还能看清 */
.tile-scrim {
  position: absolute;
  inset: 0;
  z-index: 1;
  display: block;
  border-radius: inherit;
  pointer-events: none;
  background:
    linear-gradient(
      180deg,
      rgba(6, 10, 16, 0.14) 0%,
      rgba(6, 10, 16, 0.52) 52%,
      rgba(4, 8, 13, 0.9) 100%
    ),
    radial-gradient(120% 92% at 0% 0%, rgba(6, 10, 16, 0.5), transparent 64%);
  transition: opacity 0.25s ease;
}

.tile-bg-btn {
  position: absolute;
  top: 9px;
  right: 9px;
  z-index: 20;
  display: grid;
  place-items: center;
  width: 25px;
  height: 25px;
  padding: 0;
  border-radius: 8px;
  border: 1px solid rgba(126, 160, 199, 0.3);
  background: rgba(8, 14, 22, 0.74);
  color: #b9d3ea;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07), 0 8px 18px -12px rgba(0, 0, 0, 0.95);
  opacity: 0;
  transform: translateY(-3px) scale(0.94);
  pointer-events: none;
  -webkit-backdrop-filter: blur(6px);
  backdrop-filter: blur(6px);
  transition: opacity 0.22s ease, transform 0.22s var(--ease), color 0.2s ease,
    border-color 0.2s ease, background 0.2s ease;
}

.tile:hover .tile-bg-btn,
.tile-bg-btn.is-active,
.tile-bg-btn:focus-visible {
  opacity: 1;
  transform: none;
  pointer-events: auto;
}

.tile-bg-btn.is-on {
  color: #7cc6ff;
  border-color: rgba(0, 149, 255, 0.45);
}

.tile-bg-btn:hover {
  transform: none;
  color: #eaf5ff;
  border-color: rgba(0, 149, 255, 0.65);
  background: rgba(0, 58, 108, 0.78);
  box-shadow: 0 0 0 1px rgba(0, 149, 255, 0.3), 0 10px 22px -10px rgba(0, 149, 255, 0.8);
}

/* 触屏没有 hover，入口常驻，否则这类设备根本打不开设置 */
@media (hover: none) {
  .tile-bg-btn {
    opacity: 1;
    transform: none;
    pointer-events: auto;
  }
}


.free-banner {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin: 0 0 16px;
  padding: 10px 14px;
  border: 1px solid rgba(62, 166, 255, 0.26);
  border-radius: 12px;
  background: linear-gradient(90deg, rgba(62, 166, 255, 0.1), rgba(62, 166, 255, 0.015));
  font-size: 12.5px;
  line-height: 1.55;
}
.free-banner strong {
  display: block;
  color: #e8f3ff;
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.2px;
}
.free-banner .free-text span {
  color: #8fb2d4;
}
.free-dot {
  flex: 0 0 auto;
  width: 7px;
  height: 7px;
  margin-top: 6px;
  border-radius: 50%;
  background: #48e0a4;
  box-shadow: 0 0 8px rgba(72, 224, 164, 0.7);
}
</style>
