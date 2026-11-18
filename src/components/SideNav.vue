<script setup lang="ts">
import type { Account } from "../api";

type ViewKey = "home" | "download" | "account" | "settings";

defineProps<{
  active: ViewKey;
  account: Account | null;
}>();

const emit = defineEmits<{
  (e: "navigate", view: ViewKey): void;
}>();

const items: Array<{ key: ViewKey; label: string; hint: string; icon: string }> = [
  { key: "home", label: "启动", hint: "实例大厅", icon: "M3 11.2 12 4l9 7.2V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z" },
  { key: "download", label: "下载", hint: "版本与整合包", icon: "M12 3v11m0 0 4-4m-4 4-4-4M4 20h16" },
  { key: "account", label: "账号", hint: "微软正版", icon: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zm-8 8a8 8 0 0 1 16 0" },
  { key: "settings", label: "设置", hint: "Java 与内存", icon: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm8.4-3a8.4 8.4 0 0 0-.1-1.2l2-1.5-2-3.4-2.3 1a8 8 0 0 0-2-1.2L15.6 3H8.4l-.4 2.5a8 8 0 0 0-2 1.2l-2.3-1-2 3.4 2 1.5a8.4 8.4 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-1a8 8 0 0 0 2 1.2l.4 2.5h7.2l.4-2.5a8 8 0 0 0 2-1.2l2.3 1 2-3.4-2-1.5c.06-.4.1-.8.1-1.2z" },
];

function hueOf(uuid: string): number {
  let sum = 0;
  for (const ch of uuid) sum = (sum + ch.charCodeAt(0)) % 360;
  return sum;
}
</script>

<template>
  <aside class="side">
    <div class="brand">
      <div class="brand-mark">L</div>
      <div class="brand-text">
        <strong>Love-Launcher</strong>
        <span>for Minecraft</span>
      </div>
    </div>

    <nav class="nav">
      <button
        v-for="item in items"
        :key="item.key"
        class="nav-item"
        :class="{ active: active === item.key }"
        @click="emit('navigate', item.key)"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="item.icon" />
        </svg>
        <span class="nav-label">{{ item.label }}</span>
        <span class="nav-hint">{{ item.hint }}</span>
      </button>
    </nav>

    <button class="account-chip" @click="emit('navigate', 'account')">
      <span
        v-if="account"
        class="avatar"
        :style="{ background: `linear-gradient(135deg, hsl(${hueOf(account.uuid)} 70% 52%), hsl(${(hueOf(account.uuid) + 40) % 360} 70% 38%))` }"
      >
        {{ account.name.slice(0, 1).toUpperCase() }}
      </span>
      <span v-else class="avatar empty">?</span>
      <span class="account-meta">
        <strong>{{ account ? account.name : "未登录" }}</strong>
        <em>{{ account ? "微软账号已连接" : "点击绑定正版账号" }}</em>
      </span>
    </button>
  </aside>
</template>

<style scoped>
.side {
  width: 216px;
  flex: 0 0 216px;
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 22px 16px 18px;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.01));
  border-right: 1px solid var(--line);
  backdrop-filter: blur(18px);
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 6px;
}

.brand-mark {
  width: 34px;
  height: 34px;
  border-radius: 11px;
  display: grid;
  place-items: center;
  font-weight: 700;
  font-size: 17px;
  color: #06120a;
  background: linear-gradient(135deg, var(--grass), var(--grass-deep));
  box-shadow: 0 8px 20px rgba(90, 216, 111, 0.28);
}

.brand-text {
  display: flex;
  flex-direction: column;
  line-height: 1.25;
}

.brand-text strong {
  font-size: 13px;
  letter-spacing: 0.2px;
}

.brand-text span {
  font-size: 10px;
  color: var(--text-faint);
  letter-spacing: 0.6px;
  text-transform: uppercase;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.nav-item {
  display: grid;
  grid-template-columns: 18px 1fr;
  grid-template-rows: auto auto;
  align-items: center;
  gap: 0 10px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  text-align: left;
  color: var(--text-dim);
}

.nav-item svg {
  grid-row: span 2;
  width: 18px;
  height: 18px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.nav-item .nav-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.nav-item .nav-hint {
  font-size: 10px;
  color: var(--text-faint);
  letter-spacing: 0.2px;
}

.nav-item:hover:not(.active) {
  background: rgba(255, 255, 255, 0.04);
  border-color: var(--line);
}

.nav-item.active {
  background: linear-gradient(135deg, rgba(90, 216, 111, 0.16), rgba(90, 216, 111, 0.05));
  border-color: rgba(90, 216, 111, 0.32);
  color: var(--grass);
}

.nav-item.active .nav-label {
  color: var(--grass);
}

.account-chip {
  margin-top: auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.03);
  text-align: left;
}

.avatar {
  width: 32px;
  height: 32px;
  flex: 0 0 32px;
  border-radius: 10px;
  display: grid;
  place-items: center;
  font-size: 14px;
  font-weight: 700;
  color: #06120a;
}

.avatar.empty {
  background: rgba(255, 255, 255, 0.08);
  color: var(--text-faint);
}

.account-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
  line-height: 1.3;
}

.account-meta strong {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.account-meta em {
  font-size: 10px;
  font-style: normal;
  color: var(--text-faint);
}
/* ===== 像素风侧栏 ===== */
aside {
  background: linear-gradient(180deg, #1c2530 0%, #121922 100%);
  border-right: 2px solid #0a0e12;
}

.brand-mark {
  border-radius: 3px;
  font-weight: 800;
  color: #10240f;
  background: linear-gradient(180deg, #7ee06f 0 44%, #6b4a2b 44% 100%);
  box-shadow: inset 0 2px 0 rgba(255, 255, 255, 0.3), inset 0 -3px 0 rgba(0, 0, 0, 0.35),
    0 3px 0 rgba(0, 0, 0, 0.45);
}

.brand-text strong {
  letter-spacing: 0.5px;
}

.nav-item {
  border: 2px solid transparent;
  border-radius: 4px;
}

.nav-item.active {
  position: relative;
  background: linear-gradient(180deg, rgba(98, 204, 85, 0.22), rgba(47, 122, 51, 0.12));
  border-color: rgba(98, 204, 85, 0.45);
}

.nav-item.active::before {
  content: "";
  position: absolute;
  left: -2px;
  top: 6px;
  bottom: 6px;
  width: 4px;
  background: var(--grass);
  box-shadow: 0 0 8px rgba(98, 204, 85, 0.6);
}

.account-chip {
  border-radius: 4px;
  border: 2px solid #0a0e12;
  background: #10161d;
  box-shadow: inset 0 2px 0 rgba(255, 255, 255, 0.06);
}

.account-chip:hover {
  border-color: rgba(98, 204, 85, 0.45);
}
</style>
