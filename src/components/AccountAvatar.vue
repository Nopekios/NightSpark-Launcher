<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api, type Account } from "../api";

/**
 * 账号头像：Minecraft 皮肤的那张 8x8 脸（后端裁好放大成 64x64 的 PNG）。
 * 图没到位之前先显示渐变 + 首字母占位，避免布局跳动。
 */
const props = withDefaults(
  defineProps<{
    account: Account | null;
    size?: number;
    radius?: number;
  }>(),
  { size: 32, radius: 10 }
);

/** 同一 uuid 只拉一次，切页面、切账号都复用。 */
const cache = new Map<string, string>();
const url = ref("");
const loading = ref(false);

const hue = computed(() => {
  const uuid = props.account?.uuid ?? "";
  let hash = 0;
  for (const ch of uuid) hash = (hash * 31 + ch.charCodeAt(0)) % 360;
  return hash;
});

const shellStyle = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  borderRadius: `${props.radius}px`,
  fontSize: `${Math.max(10, Math.round(props.size * 0.4))}px`,
  background: props.account
    ? `linear-gradient(150deg, hsl(${hue.value} 78% 58%) 0%, hsl(${hue.value} 72% 44%) 60%, hsl(${(hue.value + 24) % 360} 70% 32%) 100%)`
    : "rgba(126, 160, 199, 0.1)",
}));

async function load(uuid: string) {
  if (cache.has(uuid)) {
    url.value = cache.get(uuid) as string;
    return;
  }
  url.value = "";
  if (!uuid) return;
  loading.value = true;
  try {
    const data = await api.accountAvatar(uuid);
    cache.set(uuid, data);
    if (props.account?.uuid === uuid) url.value = data;
  } catch {
    // 拉不到头像不影响使用，留着占位底图
  } finally {
    loading.value = false;
  }
}

watch(
  () => props.account?.uuid,
  (uuid) => {
    if (uuid) void load(uuid);
    else url.value = "";
  },
  { immediate: true }
);
</script>

<template>
  <span class="face-shell" :style="shellStyle">
    <img
      v-if="url"
      class="face-img"
      :src="url"
      :alt="account?.name ?? ''"
      draggable="false"
    />
    <span v-else class="face-fallback">{{ account ? account.name.slice(0, 1).toUpperCase() : "?" }}</span>
  </span>
</template>

<style scoped>
.face-shell {
  display: grid;
  place-items: center;
  flex: none;
  overflow: hidden;
  font-weight: 800;
  color: #05121c;
  border: 1px solid rgba(255, 255, 255, 0.26);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.32);
}

.face-img {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: cover;
  /* 像素脸放大不能糊成一片，保持方块感 */
  image-rendering: pixelated;
  -ms-interpolation-mode: nearest-neighbor;
}

.face-fallback {
  line-height: 1;
}
</style>
