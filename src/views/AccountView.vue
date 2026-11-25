<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errText, type Account } from "../api";

const props = defineProps<{ account: Account | null }>();

const emit = defineEmits<{
  (e: "changed", account: Account | null): void;
  (e: "notify", text: string, tone: "ok" | "err"): void;
}>();

type Phase = "idle" | "waiting" | "exchanging" | "error";

const { t } = useI18n();

/** 本地账号库（可以多个）。 */
const accounts = ref<Account[]>([]);
const phase = ref<Phase>("idle");
const progress = ref("");
const errorText = ref("");
const refreshing = ref(false);
/** 正在切换到的账号 uuid，用来给按钮做 loading 态。 */
const switching = ref("");
/** 已经有账号时，点「添加账号」才展开登录面板。 */
const adding = ref(false);
let unlisteners: UnlistenFn[] = [];

const showLogin = computed(() => accounts.value.length === 0 || adding.value);

function expireTextOf(acc: Account) {
  if (!acc.expiresAt) return t("account.expireUnknown");
  const left = acc.expiresAt - Math.floor(Date.now() / 1000);
  if (left <= 0) return t("account.expired");
  const minutes = Math.floor(left / 60);
  if (minutes < 60) return t("account.expiresInMinutes", { n: minutes });
  return t("account.expiresInHours", { n: Math.floor(minutes / 60) });
}

/** 用 uuid 算一个稳定色相，多个账号的头像颜色不撞。 */
function hueOf(uuid: string) {
  let hash = 0;
  for (const ch of uuid) hash = (hash * 31 + ch.charCodeAt(0)) % 360;
  return hash;
}

function faceStyle(uuid: string) {
  const hue = hueOf(uuid);
  return {
    background: `linear-gradient(150deg, hsl(${hue} 78% 58%) 0%, hsl(${hue} 72% 44%) 60%, hsl(${(hue + 24) % 360} 70% 32%) 100%)`,
  };
}

function cleanup() {
  for (const off of unlisteners) off();
  unlisteners = [];
}

function fail(message: string) {
  phase.value = "error";
  errorText.value = message;
  progress.value = "";
  emit("notify", message, "err");
}

function applyAccounts(next: Account[]) {
  accounts.value = next;
  const active = next.find((a) => a.active) ?? null;
  emit("changed", active);
}

async function loadAccounts() {
  try {
    applyAccounts(await api.listAccounts());
  } catch (err) {
    emit("notify", errText(err), "err");
  }
}

async function exchange(code: string) {
  phase.value = "exchanging";
  try {
    const account = await api.exchangeCode(code);
    phase.value = "idle";
    progress.value = "";
    adding.value = false;
    await loadAccounts();
    emit("notify", t("account.loggedIn", { name: account.name }), "ok");
  } catch (err) {
    fail(errText(err));
  } finally {
    cleanup();
  }
}

async function beginLogin() {
  if (phase.value === "waiting" || phase.value === "exchanging") return;
  cleanup();
  phase.value = "waiting";
  errorText.value = "";
  progress.value = t("account.opening");

  unlisteners.push(
    await listen<string>("login-progress", (event) => {
      progress.value = event.payload;
    })
  );
  unlisteners.push(
    await listen<string>("msa-code", (event) => {
      progress.value = t("account.exchanging");
      void exchange(event.payload);
    })
  );
  unlisteners.push(
    await listen<string>("msa-error", (event) => {
      fail(event.payload);
      cleanup();
    })
  );

  try {
    await api.startLogin();
  } catch (err) {
    fail(errText(err));
    cleanup();
  }
}

/** 登录面板里点「取消」：已经有账号时收起面板即可。 */
function cancelAdd() {
  cleanup();
  adding.value = false;
  phase.value = "idle";
  progress.value = "";
  errorText.value = "";
}

async function refresh() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    await api.refreshAccount();
    await loadAccounts();
    emit("notify", t("account.refreshed"), "ok");
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    refreshing.value = false;
  }
}

async function switchTo(acc: Account) {
  if (acc.active || switching.value) return;
  switching.value = acc.uuid;
  try {
    await api.switchAccount(acc.uuid);
    await loadAccounts();
    emit("notify", t("account.switched", { name: acc.name }), "ok");
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    switching.value = "";
  }
}

async function remove(acc: Account) {
  try {
    await api.removeAccount(acc.uuid);
    await loadAccounts();
    emit("notify", t("account.removed", { name: acc.name }), "ok");
  } catch (err) {
    emit("notify", errText(err), "err");
  }
}

onMounted(async () => {
  await loadAccounts();
  // 后台自动续期、其它窗口登录都会推这个事件，列表跟着刷新。
  unlisteners.push(await api.onAccountsChanged(applyAccounts));
});

onUnmounted(cleanup);
</script>

<template>
  <section class="page">
    <header class="page-head">
      <div>
        <h1>{{ t("account.title") }}</h1>
        <p>{{ t("account.subtitle") }}</p>
      </div>
      <span v-if="accounts.length" class="tag">{{ t("account.connected") }}</span>
      <span v-else class="tag muted">{{ t("account.notSignedIn") }}</span>
    </header>

    <div v-if="!showLogin" class="panel accounts">
      <article
        v-for="acc in accounts"
        :key="acc.uuid"
        class="acc-row"
        :class="{ on: acc.active }"
      >
        <div class="face" :style="faceStyle(acc.uuid)">
          {{ acc.name.slice(0, 1).toUpperCase() }}
        </div>
        <div class="acc-main">
          <div class="acc-title">
            <h2>{{ acc.name }}</h2>
            <span v-if="acc.active" class="badge">{{ t("account.current") }}</span>
          </div>
          <p class="uuid">{{ acc.uuid }}</p>
          <dl class="facts">
            <div>
              <dt>{{ t("account.tokenStatus") }}</dt>
              <dd>{{ expireTextOf(acc) }}</dd>
            </div>
            <div>
              <dt>{{ t("account.refreshToken") }}</dt>
              <dd>{{ acc.hasRefreshToken ? t("account.refreshSaved") : t("account.refreshMissing") }}</dd>
            </div>
          </dl>
        </div>
        <div class="acc-actions">
          <button
            v-if="!acc.active"
            class="ghost"
            :disabled="switching === acc.uuid"
            @click="switchTo(acc)"
          >
            {{ switching === acc.uuid ? t("account.switching") : t("account.switchBtn") }}
          </button>
          <button v-else :disabled="refreshing" @click="refresh">
            {{ refreshing ? t("account.refreshing") : t("account.refreshNow") }}
          </button>
          <button class="ghost-danger" @click="remove(acc)">{{ t("account.removeBtn") }}</button>
        </div>
      </article>

      <div class="accounts-foot">
        <button class="primary" @click="adding = true">{{ t("account.addAccount") }}</button>
        <p class="hint">{{ t("account.autoRenewHint") }}</p>
      </div>
    </div>

    <div v-else class="panel login">
      <h2>{{ t("account.bindTitle") }}</h2>
      <p>
        {{ t("account.bindLine1") }}<br />
        {{ t("account.bindLine2") }}
      </p>
      <div class="login-actions">
        <button
          class="primary"
          :disabled="phase === 'waiting' || phase === 'exchanging'"
          @click="beginLogin"
        >
          {{ phase === "waiting" || phase === "exchanging" ? t("account.signingIn") : t("account.signIn") }}
        </button>
        <button v-if="accounts.length" class="ghost" @click="cancelAdd">
          {{ t("account.cancelAdd") }}
        </button>
      </div>
      <p v-if="progress" class="progress">{{ progress }}</p>
      <p v-if="phase === 'error'" class="error">{{ errorText }}</p>
      <ul class="tips">
        <li>{{ t("account.tip1") }}</li>
        <li>{{ t("account.tip2") }}</li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
/* ── 账号列表 ── */
.accounts {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
  max-width: 860px;
  margin: 0 auto;
  padding: 18px;
}

.acc-row {
  display: flex;
  align-items: flex-start;
  gap: 16px;
  padding: 16px 18px;
  border-radius: 16px;
  border: 1px solid rgba(126, 160, 199, 0.14);
  background: rgba(255, 255, 255, 0.025);
  transition: border-color 0.18s ease, background 0.18s ease;
}

.acc-row.on {
  border-color: rgba(62, 177, 255, 0.55);
  background: rgba(0, 149, 255, 0.07);
  box-shadow: 0 18px 40px -26px rgba(0, 149, 255, 0.9);
}

.face {
  display: grid;
  place-items: center;
  width: 56px;
  height: 56px;
  flex: none;
  border-radius: 16px;
  font-size: 22px;
  font-weight: 800;
  color: #f4fbff;
  border: 1px solid rgba(255, 255, 255, 0.22);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4), 0 14px 30px -16px rgba(0, 149, 255, 0.8);
  text-shadow: 0 1px 3px rgba(0, 28, 56, 0.6);
}

.acc-main {
  flex: 1;
  min-width: 0;
}

.acc-title {
  display: flex;
  align-items: center;
  gap: 10px;
}

.acc-title h2 {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: -0.02em;
  line-height: 1.2;
}

.badge {
  padding: 2px 9px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.06em;
  color: #cfeaff;
  background: rgba(0, 149, 255, 0.18);
  border: 1px solid rgba(62, 177, 255, 0.42);
}

.uuid {
  margin: 5px 0 0;
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
  letter-spacing: 0.02em;
  color: var(--text-faint);
  word-break: break-all;
}

.facts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 8px;
  margin: 12px 0 0;
}

.facts div {
  padding: 9px 12px;
  border-radius: 11px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: rgba(255, 255, 255, 0.025);
}

.facts dt {
  margin-bottom: 3px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.facts dd {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.acc-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: none;
}

.accounts-foot {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 6px;
}

.accounts-foot .hint {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--text-faint);
}

/* ── 登录面板 ── */
.login {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: 100%;
  max-width: 760px;
  margin: 0 auto;
  padding: 30px 32px;
}

.login h2 {
  margin: 0;
  font-size: 22px;
  font-weight: 700;
  letter-spacing: -0.025em;
  line-height: 1.2;
}

.login p {
  margin: 0;
  max-width: 56ch;
  font-size: 12.5px;
  line-height: 1.75;
  color: var(--text-dim);
}

.login-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.login .progress {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  font-size: 12px;
  color: #9fd0ff;
}

.login .progress::before {
  content: "";
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background: #3eb1ff;
  box-shadow: 0 0 10px rgba(0, 149, 255, 0.9);
  animation: ll-glow-pulse 1.4s ease-in-out infinite;
}

.login .error {
  padding: 11px 14px;
  border-radius: 11px;
  border: 1px solid rgba(255, 90, 110, 0.34);
  background: rgba(255, 90, 110, 0.07);
  font-size: 12px;
  color: #ffc2ca;
}

.tips {
  display: flex;
  flex-direction: column;
  gap: 7px;
  margin: 2px 0 0;
  padding: 0;
  list-style: none;
}

.tips li {
  position: relative;
  padding-left: 16px;
  font-size: 11.5px;
  line-height: 1.65;
  color: var(--text-faint);
}

.tips li::before {
  content: "";
  position: absolute;
  left: 0;
  top: 7px;
  width: 5px;
  height: 5px;
  border-radius: 999px;
  background: rgba(0, 149, 255, 0.7);
  box-shadow: 0 0 8px rgba(0, 149, 255, 0.6);
}
</style>
