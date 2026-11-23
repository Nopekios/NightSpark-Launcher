<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

type BgType = "none" | "image" | "gradient";

interface BgConfig {
  type: BgType;
  image: string;
  color: string;
  opacity: number;
  blur: number;
  scrim: number;
  position: string;
}

const props = defineProps<{
  config: BgConfig;
  anchor: { x: number; y: number };
  name: string;
  hint?: string;
}>();

const emit = defineEmits<{
  (e: "update", patch: Partial<BgConfig>): void;
  (e: "reset"): void;
  (e: "close"): void;
}>();

const { t } = useI18n();

// 浮层宽度与预估高度：只用来算落点和翻转，真实高度由内容决定
const PANEL_W = 292;
const EST_H = 452;

const pos = ref({ x: props.anchor.x, y: props.anchor.y });
const panelEl = ref<HTMLElement | null>(null);

function place() {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  // 类型切换会改变面板高度，能量到就用实测值，量不到退回预估值
  const measured = panelEl.value?.offsetHeight ?? 0;
  const h = Math.min(measured > 0 ? measured : EST_H, vh - 24);
  // 入口按钮在磁贴右上角：浮层贴着按钮左边缘，右边距不够就整体回收
  let x = props.anchor.x - PANEL_W + 26;
  x = Math.max(10, Math.min(x, vw - PANEL_W - 10));
  // 下方放不下就翻到按钮上方
  let y = props.anchor.y;
  if (y + h > vh - 10) y = props.anchor.y - h - 46;
  y = Math.max(10, Math.min(y, vh - h - 10));
  pos.value = { x, y };
}

function onDocPointerDown(e: PointerEvent) {
  const target = e.target as HTMLElement | null;
  if (!target) return;
  // 点面板内部不关；点入口按钮交给按钮自己的开关逻辑，避免关了又开
  if (target.closest(".bg-panel") || target.closest(".tile-bg-btn")) return;
  emit("close");
}

function onKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}

onMounted(() => {
  place();
  nextTick(place);
  window.addEventListener("resize", place);
  window.addEventListener("scroll", place, true);
  document.addEventListener("pointerdown", onDocPointerDown, true);
  document.addEventListener("keydown", onKeyDown);
});

// 换背景类型后面板高度会变，重新找落点避免贴着屏幕底边出不来
watch(
  () => props.config.type,
  () => nextTick(place)
);

onBeforeUnmount(() => {
  window.removeEventListener("resize", place);
  window.removeEventListener("scroll", place, true);
  document.removeEventListener("pointerdown", onDocPointerDown, true);
  document.removeEventListener("keydown", onKeyDown);
});

const typeOptions = computed<{ value: BgType; label: string }[]>(() => [
  { value: "none", label: t("bg.typeNone") },
  { value: "image", label: t("bg.typeImage") },
  { value: "gradient", label: t("bg.typeGradient") },
]);

const gradientPresets = [
  "linear-gradient(135deg, #0d2438 0%, #0a1524 60%, #12324d 100%)",
  "linear-gradient(135deg, #1b1030 0%, #0a0f1c 62%, #2a1140 100%)",
  "linear-gradient(135deg, #06231f 0%, #07131a 60%, #0d3a30 100%)",
  "linear-gradient(135deg, #2a1206 0%, #140b08 60%, #43200a 100%)",
  "radial-gradient(120% 120% at 82% 0%, #16324a 0%, #0a111b 58%, #070c13 100%)",
];

const solidPresets = ["#0d2438", "#132a44", "#1b1030", "#122a24", "#2a1206", "#101720"];

const positionGrid = [
  ["0% 0%", "50% 0%", "100% 0%"],
  ["0% 50%", "50% 50%", "100% 50%"],
  ["0% 100%", "50% 100%", "100% 100%"],
];

const positionCells = positionGrid.flat();

const POSITION_KEYS: Record<string, string> = {
  "0% 0%": "bg.posTopLeft",
  "50% 0%": "bg.posTop",
  "100% 0%": "bg.posTopRight",
  "0% 50%": "bg.posLeft",
  "50% 50%": "bg.posCenter",
  "100% 50%": "bg.posRight",
  "0% 100%": "bg.posBottomLeft",
  "50% 100%": "bg.posBottom",
  "100% 100%": "bg.posBottomRight",
};

function positionText(value: string) {
  return t(POSITION_KEYS[value] ?? "bg.posCenter");
}

const fileInput = ref<HTMLInputElement | null>(null);
const reading = ref(false);
const localHint = ref("");

const hintText = computed(() => localHint.value || props.hint || "");

const solidColor = computed(() =>
  /^#[0-9a-fA-F]{6}$/.test(props.config.color) ? props.config.color : "#0d2438"
);

const thumbStyle = computed<Record<string, string>>(() => {
  const style: Record<string, string> = {};
  if (props.config.image) style.backgroundImage = `url(${props.config.image})`;
  return style;
});

function setType(value: BgType) {
  localHint.value = "";
  emit("update", { type: value });
  // 选「图片」但还没有底图时直接把文件选择器弹出来，少点一次
  if (value === "image" && !props.config.image) pickFile();
}

function pickFile() {
  fileInput.value?.click();
}

function onRange(key: "opacity" | "blur" | "scrim", e: Event) {
  const raw = Number((e.target as HTMLInputElement).value);
  if (!Number.isFinite(raw)) return;
  emit("update", { [key]: raw } as Partial<BgConfig>);
}

function onColor(e: Event) {
  const value = (e.target as HTMLInputElement).value;
  if (!value) return;
  emit("update", { color: value });
}

async function onFileChange(e: Event) {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  if (!file.type.startsWith("image/")) {
    localHint.value = t("bg.onlyImages");
    return;
  }
  reading.value = true;
  localHint.value = "";
  try {
    const dataUrl = await compressToDataUrl(file);
    emit("update", { type: "image", image: dataUrl });
  } catch (err) {
    localHint.value = t("bg.imageUnreadable");
    console.warn("[instance-bg] 图片压缩失败", err);
  } finally {
    reading.value = false;
  }
}

// canvas 压到最大宽 1600、jpeg 0.85：既够铺满磁贴，又不至于把 localStorage 撑爆
function compressToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const objectUrl = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      try {
        const naturalW = img.naturalWidth || img.width;
        const naturalH = img.naturalHeight || img.height;
        if (!naturalW || !naturalH) throw new Error("empty image");
        const MAX_W = 1600;
        const scale = Math.min(1, MAX_W / naturalW);
        const width = Math.max(1, Math.round(naturalW * scale));
        const height = Math.max(1, Math.round(naturalH * scale));
        const canvas = document.createElement("canvas");
        canvas.width = width;
        canvas.height = height;
        const ctx = canvas.getContext("2d");
        if (!ctx) throw new Error("no 2d context");
        ctx.imageSmoothingEnabled = true;
        ctx.imageSmoothingQuality = "high";
        // 透明 PNG 直接转 jpeg 会变黑，先垫一层深色底
        ctx.fillStyle = "#0a1220";
        ctx.fillRect(0, 0, width, height);
        ctx.drawImage(img, 0, 0, width, height);
        resolve(canvas.toDataURL("image/jpeg", 0.85));
      } catch (err) {
        reject(err);
      } finally {
        URL.revokeObjectURL(objectUrl);
      }
    };
    img.onerror = () => {
      URL.revokeObjectURL(objectUrl);
      reject(new Error("image decode failed"));
    };
    img.src = objectUrl;
  });
}

function onReset() {
  localHint.value = "";
  emit("reset");
}
</script>

<template>
  <Teleport to="body">
    <div
      ref="panelEl"
      class="bg-panel"
      :style="{ left: `${pos.x}px`, top: `${pos.y}px`, width: `${PANEL_W}px` }"
      @pointerdown.stop
      @click.stop
      @mousedown.stop
      @contextmenu.stop
    >
      <header class="bg-head">
        <span class="bg-title">
          <span class="bg-dot" aria-hidden="true" />
          {{ t("bg.title") }}
        </span>
        <button class="bg-x" type="button" :title="t('bg.close')" @click="emit('close')">×</button>
      </header>

      <p class="bg-name" :title="name">{{ name }}</p>

      <div class="bg-types">
        <button
          v-for="opt in typeOptions"
          :key="opt.value"
          type="button"
          class="bg-type"
          :class="{ on: config.type === opt.value }"
          @click="setType(opt.value)"
        >
          {{ opt.label }}
        </button>
      </div>

      <div v-if="config.type === 'image'" class="bg-block">
        <div class="bg-pick">
          <span class="bg-thumb" :style="thumbStyle">
            <span v-if="!config.image" class="bg-thumb-empty">{{ t("bg.noImage") }}</span>
          </span>
          <button type="button" class="bg-file" :disabled="reading" @click="pickFile">
            {{ reading ? t("bg.reading") : config.image ? t("bg.changeImage") : t("bg.pickImage") }}
          </button>
        </div>
        <div class="bg-pos">
          <span class="row-label">{{ t("bg.position") }}<b>{{ positionText(config.position) }}</b></span>
          <div class="bg-pos-grid">
            <button
              v-for="cell in positionCells"
              :key="cell"
              type="button"
              class="bg-pos-cell"
              :class="{ on: config.position === cell }"
              :title="positionText(cell)"
              @click="emit('update', { position: cell })"
            />
          </div>
        </div>
      </div>

      <div v-else-if="config.type === 'gradient'" class="bg-block">
        <div class="row-label">{{ t("bg.preset") }}</div>
        <div class="bg-swatches">
          <button
            v-for="preset in gradientPresets"
            :key="preset"
            type="button"
            class="bg-swatch"
            :class="{ on: config.color === preset }"
            :style="{ background: preset }"
            :title="t('bg.gradient')"
            @click="emit('update', { color: preset })"
          />
          <button
            v-for="preset in solidPresets"
            :key="preset"
            type="button"
            class="bg-swatch"
            :class="{ on: config.color === preset }"
            :style="{ background: preset }"
            :title="preset"
            @click="emit('update', { color: preset })"
          />
          <label class="bg-swatch bg-swatch-pick" :title="t('bg.customColor')">
            <span class="bg-swatch-inner" :style="{ background: solidColor }" />
            <input type="color" :value="solidColor" @input="onColor" />
          </label>
        </div>
      </div>

      <div v-if="config.type !== 'none'" class="bg-sliders">
        <div class="row">
          <span class="row-label">{{ t("bg.opacity") }}<b>{{ config.opacity }}%</b></span>
          <input
            class="slider"
            type="range"
            min="0"
            max="100"
            step="1"
            :value="config.opacity"
            @input="onRange('opacity', $event)"
          />
        </div>
        <div class="row">
          <span class="row-label">{{ t("bg.blur") }}<b>{{ config.blur }}px</b></span>
          <input
            class="slider"
            type="range"
            min="0"
            max="30"
            step="1"
            :value="config.blur"
            @input="onRange('blur', $event)"
          />
        </div>
        <div class="row">
          <span class="row-label">{{ t("bg.scrim") }}<b>{{ config.scrim }}%</b></span>
          <input
            class="slider"
            type="range"
            min="0"
            max="90"
            step="1"
            :value="config.scrim"
            @input="onRange('scrim', $event)"
          />
        </div>
      </div>

      <p v-if="config.type === 'none'" class="bg-tip">{{ t("bg.keepDefault") }}</p>
      <p v-if="hintText" class="bg-tip warn">{{ hintText }}</p>

      <footer class="bg-foot">
        <button type="button" class="bg-reset" @click="onReset">{{ t("bg.reset") }}</button>
        <button type="button" class="bg-done" @click="emit('close')">{{ t("bg.done") }}</button>
      </footer>

      <input
        ref="fileInput"
        class="bg-hidden"
        type="file"
        accept="image/*"
        @change="onFileChange"
      />
    </div>
  </Teleport>
</template>

<style scoped>
.bg-panel {
  position: fixed;
  z-index: 85;
  display: flex;
  flex-direction: column;
  gap: 11px;
  padding: 13px 14px 12px;
  border-radius: 14px;
  border: 1px solid rgba(126, 160, 199, 0.22);
  background: linear-gradient(180deg, rgba(16, 25, 38, 0.98) 0%, rgba(9, 14, 22, 0.99) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 28px 60px -26px rgba(0, 0, 0, 1);
  -webkit-backdrop-filter: blur(14px) saturate(120%);
  backdrop-filter: blur(14px) saturate(120%);
  color: var(--text);
  font-size: 12px;
  animation: bg-panel-in 0.18s var(--ease-expo, cubic-bezier(0.22, 1, 0.36, 1)) both;
}

@keyframes bg-panel-in {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.985);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

.bg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.bg-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.bg-dot {
  width: 6px;
  height: 6px;
  border-radius: 2px;
  background: var(--accent);
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.9);
}

.bg-x {
  width: 20px;
  height: 20px;
  padding: 0;
  display: grid;
  place-items: center;
  font-size: 14px;
  line-height: 1;
  border-radius: 6px;
  color: var(--text-dim);
  background: transparent;
  border: 1px solid transparent;
  box-shadow: none;
}

.bg-x:hover {
  color: var(--text);
  border-color: rgba(126, 160, 199, 0.28);
  background: rgba(255, 255, 255, 0.04);
  transform: none;
  box-shadow: none;
}

.bg-name {
  margin: -4px 0 0;
  font-size: 11px;
  color: var(--text-faint);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.bg-types {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 3px;
  border-radius: 10px;
  border: 1px solid rgba(126, 160, 199, 0.16);
  background: rgba(6, 11, 18, 0.7);
}

.bg-type {
  padding: 6px 4px;
  font-size: 11px;
  font-weight: 650;
  border-radius: 7px;
  border: 1px solid transparent;
  background: transparent;
  box-shadow: none;
  color: var(--text-dim);
}

.bg-type:hover:not(:disabled) {
  color: var(--text);
  background: rgba(255, 255, 255, 0.05);
  border-color: transparent;
  transform: none;
  box-shadow: none;
}

.bg-type.on {
  color: #eaf5ff;
  border-color: rgba(0, 149, 255, 0.5);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.28), rgba(0, 114, 198, 0.18));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.16);
}

.bg-block {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.bg-pick {
  display: flex;
  align-items: center;
  gap: 9px;
}

.bg-thumb {
  position: relative;
  width: 58px;
  height: 38px;
  flex: 0 0 auto;
  border-radius: 8px;
  border: 1px solid rgba(126, 160, 199, 0.22);
  background: rgba(8, 14, 22, 0.9) center / cover no-repeat;
  overflow: hidden;
}

.bg-thumb-empty {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  font-size: 10px;
  color: var(--text-faint);
}

.bg-file {
  flex: 1 1 auto;
  padding: 7px 10px;
  font-size: 11px;
  font-weight: 650;
  border-radius: 8px;
}

.bg-pos {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.bg-pos-grid {
  display: grid;
  grid-template-columns: repeat(3, 12px);
  grid-template-rows: repeat(3, 12px);
  gap: 3px;
}

.bg-pos-cell {
  width: 12px;
  height: 12px;
  padding: 0;
  border-radius: 3px;
  border: 1px solid rgba(126, 160, 199, 0.28);
  background: rgba(255, 255, 255, 0.04);
  box-shadow: none;
}

.bg-pos-cell:hover:not(:disabled) {
  border-color: rgba(0, 149, 255, 0.6);
  background: rgba(0, 149, 255, 0.2);
  transform: none;
  box-shadow: none;
}

.bg-pos-cell.on {
  border-color: rgba(0, 149, 255, 0.9);
  background: rgba(0, 149, 255, 0.75);
  box-shadow: 0 0 8px rgba(0, 149, 255, 0.6);
}

.row-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 10px;
  font-weight: 650;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.row-label b {
  font-weight: 700;
  letter-spacing: 0;
  text-transform: none;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.bg-swatches {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.bg-swatch {
  width: 30px;
  height: 22px;
  padding: 0;
  border-radius: 6px;
  border: 1px solid rgba(126, 160, 199, 0.26);
  box-shadow: none;
  overflow: hidden;
  position: relative;
}

.bg-swatch:hover:not(:disabled) {
  transform: translateY(-1px);
  border-color: rgba(0, 149, 255, 0.6);
  box-shadow: 0 4px 12px -4px rgba(0, 149, 255, 0.6);
}

.bg-swatch.on {
  border-color: rgba(0, 149, 255, 0.95);
  box-shadow: 0 0 0 1px rgba(0, 149, 255, 0.7), 0 6px 16px -6px rgba(0, 149, 255, 0.7);
}

.bg-swatch-pick {
  display: grid;
  place-items: center;
  cursor: pointer;
  background: rgba(255, 255, 255, 0.05);
}

.bg-swatch-inner {
  width: 100%;
  height: 100%;
  border-radius: 5px;
}

.bg-swatch-pick input {
  position: absolute;
  inset: 0;
  opacity: 0;
  width: 100%;
  height: 100%;
  padding: 0;
  border: none;
  cursor: pointer;
}

.bg-sliders {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 2px;
}

.row {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.slider {
  width: 100%;
  height: 15px;
  margin: 0;
  padding: 0;
  border: none;
  background: transparent;
  accent-color: var(--accent);
  cursor: pointer;
  -webkit-appearance: none;
  appearance: none;
}

.slider::-webkit-slider-runnable-track {
  height: 3px;
  border-radius: 2px;
  background: linear-gradient(90deg, rgba(0, 149, 255, 0.55), rgba(126, 160, 199, 0.22));
}

.slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 11px;
  height: 11px;
  margin-top: -4px;
  border-radius: 3px;
  background: #dceaf8;
  border: 1px solid rgba(0, 149, 255, 0.9);
  box-shadow: 0 0 8px rgba(0, 149, 255, 0.7);
}

.bg-tip {
  margin: 0;
  font-size: 10.5px;
  line-height: 1.5;
  color: var(--text-faint);
}

.bg-tip.warn {
  color: #ffb3bd;
}

.bg-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding-top: 3px;
  border-top: 1px solid rgba(126, 160, 199, 0.14);
  margin-top: 1px;
}

.bg-reset,
.bg-done {
  padding: 6px 12px;
  font-size: 11px;
  font-weight: 650;
  border-radius: 8px;
}

.bg-reset {
  color: var(--text-dim);
  background: transparent;
  border-color: rgba(126, 160, 199, 0.24);
  box-shadow: none;
}

.bg-done {
  color: #eaf5ff;
  border-color: rgba(0, 149, 255, 0.55);
  background: linear-gradient(180deg, rgba(0, 149, 255, 0.42), rgba(0, 114, 198, 0.3));
}

.bg-hidden {
  display: none;
}
</style>
