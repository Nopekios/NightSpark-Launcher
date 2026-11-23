<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { Account } from "../api";

type ViewKey = "home" | "download" | "account" | "settings";

defineProps<{
  active: ViewKey;
  account: Account | null;
}>();

const emit = defineEmits<{
  (e: "navigate", view: ViewKey): void;
}>();

const { t } = useI18n();

const iconOf = {
  home: "M3 11.2 12 4l9 7.2V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z",
  download: "M12 3v11m0 0 4-4m-4 4-4-4M4 20h16",
  account: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zm-8 8a8 8 0 0 1 16 0",
  settings:
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm8.4-3a8.4 8.4 0 0 0-.1-1.2l2-1.5-2-3.4-2.3 1a8 8 0 0 0-2-1.2L15.6 3H8.4l-.4 2.5a8 8 0 0 0-2 1.2l-2.3-1-2 3.4 2 1.5a8.4 8.4 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-1a8 8 0 0 0 2 1.2l.4 2.5h7.2l.4-2.5a8 8 0 0 0 2-1.2l2.3 1 2-3.4-2-1.5c.06-.4.1-.8.1-1.2z",
};

const items = computed<Array<{ key: ViewKey; label: string; hint: string; icon: string }>>(() => [
  { key: "home", label: t("nav.home"), hint: t("nav.homeHint"), icon: iconOf.home },
  { key: "download", label: t("nav.download"), hint: t("nav.downloadHint"), icon: iconOf.download },
  { key: "account", label: t("nav.account"), hint: t("nav.accountHint"), icon: iconOf.account },
  { key: "settings", label: t("nav.settings"), hint: t("nav.settingsHint"), icon: iconOf.settings },
]);

function hueOf(uuid: string): number {
  let sum = 0;
  for (const ch of uuid) sum = (sum + ch.charCodeAt(0)) % 360;
  return sum;
}
</script>

<template>
  <aside class="side">
    <div class="brand">
      <img
        class="brand-mark"
        src="/nightspark.svg"
        alt="NightSpark"
        draggable="false"
      />
      <div class="brand-text">
        <strong>NightSpark</strong>
        <span>for Minecraft</span>
      </div>
    </div>

    <div class="brand-rule" />

    <span class="nav-cap">{{ t("nav.caption") }}</span>

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
        <strong>{{ account ? account.name : t("nav.notLoggedIn") }}</strong>
        <em>{{ account ? t("nav.connected") : t("nav.bindAccount") }}</em>
      </span>
    </button>
  </aside>
</template>

<style scoped>
.side {
  position: relative;
  width: 212px;
  flex: 0 0 212px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px 14px 16px;
  background: linear-gradient(180deg, rgba(19, 29, 42, 0.72) 0%, rgba(8, 13, 20, 0.82) 100%);
  border-right: 1px solid rgba(126, 160, 199, 0.1);
  -webkit-backdrop-filter: blur(18px) saturate(130%);
  backdrop-filter: blur(18px) saturate(130%);
  box-shadow: inset -1px 0 0 rgba(0, 149, 255, 0.06);
  z-index: 2;
}

.brand {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 2px 6px 0;
}

.brand-mark {
  width: 30px;
  height: 30px;
  flex: none;
  display: block;
  object-fit: contain;
  filter: drop-shadow(0 8px 18px rgba(0, 149, 255, 0.45));
  user-select: none;
  -webkit-user-drag: none;
}

.brand-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  line-height: 1.2;
  min-width: 0;
}

.brand-text strong {
  font-size: 12.5px;
  font-weight: 700;
  letter-spacing: -0.01em;
  white-space: nowrap;
}

.brand-text span {
  font-size: 9px;
  font-weight: 600;
  color: var(--text-faint);
  letter-spacing: 0.13em;
  text-transform: uppercase;
}

.brand-rule {
  height: 1px;
  margin: 4px 6px 2px;
  background: linear-gradient(90deg, transparent, rgba(126, 160, 199, 0.22) 22%, rgba(0, 149, 255, 0.3) 50%, rgba(126, 160, 199, 0.22) 78%, transparent);
}

.nav-cap {
  padding: 0 10px;
  font-size: 9.5px;
  font-weight: 700;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.nav-item {
  position: relative;
  display: grid;
  grid-template-columns: 18px 1fr;
  grid-template-rows: auto auto;
  align-items: center;
  gap: 1px 11px;
  padding: 9px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  box-shadow: none;
  text-align: left;
  color: var(--text-dim);
  transition: background 0.2s cubic-bezier(0.22, 1, 0.36, 1), color 0.2s ease,
    border-color 0.2s ease, transform 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

/* 左侧激活光条 */
.nav-item::before {
  content: "";
  position: absolute;
  left: -1px;
  top: 50%;
  width: 2px;
  height: 0;
  border-radius: 999px;
  background: linear-gradient(180deg, #3eb1ff, #0072c6);
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.9);
  transform: translateY(-50%);
  transition: height 0.28s cubic-bezier(0.22, 1, 0.36, 1);
}

.nav-item svg {
  grid-row: span 2;
  width: 18px;
  height: 18px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
  stroke-linejoin: round;
  transition: stroke-width 0.2s ease, transform 0.28s cubic-bezier(0.22, 1, 0.36, 1);
}

.nav-item .nav-label {
  font-size: 12.5px;
  font-weight: 600;
  letter-spacing: 0.005em;
  color: var(--text);
  transition: color 0.2s ease;
}

.nav-item .nav-hint {
  font-size: 10px;
  color: var(--text-faint);
  letter-spacing: 0.01em;
}

.nav-item:hover:not(.active) {
  background: rgba(126, 160, 199, 0.07);
  border-color: rgba(126, 160, 199, 0.12);
  color: var(--text);
  transform: translateX(1px);
}

.nav-item:hover:not(.active) svg {
  stroke-width: 1.9;
}

.nav-item.active {
  background: linear-gradient(100deg, rgba(0, 149, 255, 0.16) 0%, rgba(0, 149, 255, 0.03) 62%);
  border-color: rgba(0, 149, 255, 0.22);
  color: var(--accent-hi);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05), 0 8px 22px -14px rgba(0, 149, 255, 0.9);
}

.nav-item.active::before {
  height: 20px;
}

.nav-item.active .nav-label {
  color: #cfe8ff;
}

.nav-item.active .nav-hint {
  color: rgba(107, 170, 224, 0.9);
}

.nav-item.active svg {
  stroke-width: 2;
  filter: drop-shadow(0 0 6px rgba(0, 149, 255, 0.7));
}

/* 底部账号卡 */
.account-chip {
  margin-top: auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px;
  border-radius: 12px;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.055) 0%, rgba(255, 255, 255, 0.015) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05);
  text-align: left;
  overflow: hidden;
}

.account-chip:hover:not(:disabled) {
  border-color: rgba(0, 149, 255, 0.34);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08), 0 10px 26px -14px rgba(0, 149, 255, 0.7);
}

.avatar {
  width: 32px;
  height: 32px;
  flex: 0 0 32px;
  border-radius: 10px;
  display: grid;
  place-items: center;
  font-size: 13px;
  font-weight: 800;
  color: #05121c;
  border: 1px solid rgba(255, 255, 255, 0.26);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4);
}

.avatar.empty {
  color: var(--text-faint);
  background: rgba(126, 160, 199, 0.1);
  border-color: rgba(126, 160, 199, 0.18);
  box-shadow: none;
  font-weight: 700;
}

.account-meta {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  line-height: 1.25;
}

.account-meta strong {
  font-size: 12px;
  font-weight: 650;
  letter-spacing: -0.005em;
  color: var(--text);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.account-meta em {
  font-style: normal;
  font-size: 10px;
  color: var(--text-faint);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
