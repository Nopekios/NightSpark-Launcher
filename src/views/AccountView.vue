<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errText, type Account } from "../api";
import CardContainer from "@/components/ui/card-3d/CardContainer.vue";
import CardBody from "@/components/ui/card-3d/CardBody.vue";
import CardItem from "@/components/ui/card-3d/CardItem.vue";

const props = defineProps<{ account: Account | null }>();

const emit = defineEmits<{
  (e: "changed", account: Account | null): void;
  (e: "notify", text: string, tone: "ok" | "err"): void;
}>();

type Phase = "idle" | "waiting" | "exchanging" | "error";

const { t } = useI18n();

const phase = ref<Phase>("idle");
const progress = ref("");
const errorText = ref("");
const refreshing = ref(false);
let unlisteners: UnlistenFn[] = [];

const expireText = computed(() => {
  if (!props.account || !props.account.expiresAt) return t("account.expireUnknown");
  const left = props.account.expiresAt - Math.floor(Date.now() / 1000);
  if (left <= 0) return t("account.expired");
  const minutes = Math.floor(left / 60);
  if (minutes < 60) return t("account.expiresInMinutes", { n: minutes });
  return t("account.expiresInHours", { n: Math.floor(minutes / 60) });
});

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

async function exchange(code: string) {
  phase.value = "exchanging";
  try {
    const account = await api.exchangeCode(code);
    phase.value = "idle";
    progress.value = "";
    emit("changed", account);
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

async function refresh() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    const account = await api.refreshAccount();
    emit("changed", account);
    emit("notify", t("account.refreshed"), "ok");
  } catch (err) {
    emit("notify", errText(err), "err");
  } finally {
    refreshing.value = false;
  }
}

async function logout() {
  try {
    await api.logout();
    emit("changed", null);
    phase.value = "idle";
    progress.value = "";
    emit("notify", t("account.loggedOut"), "ok");
  } catch (err) {
    emit("notify", errText(err), "err");
  }
}

onUnmounted(cleanup);
</script>

<template>
  <section class="page">
    <header class="page-head">
      <div>
        <h1>{{ t("account.title") }}</h1>
        <p>{{ t("account.subtitle") }}</p>
      </div>
      <span v-if="account" class="tag">{{ t("account.connected") }}</span>
      <span v-else class="tag muted">{{ t("account.notSignedIn") }}</span>
    </header>

    <CardContainer v-if="account" class="acc-3d">
      <CardBody class="acc-3d-body">
        <CardItem :translate-z="36" class="acc-3d-item">
          <div class="panel profile">
            <div class="profile-main">
        <div class="face">{{ account.name.slice(0, 1).toUpperCase() }}</div>
        <div>
          <h2>{{ account.name }}</h2>
          <p class="uuid">{{ account.uuid }}</p>
        </div>
      </div>
      <dl class="facts">
        <div>
          <dt>{{ t("account.tokenStatus") }}</dt>
          <dd>{{ expireText }}</dd>
        </div>
        <div>
          <dt>{{ t("account.refreshToken") }}</dt>
          <dd>{{ account.hasRefreshToken ? t("account.refreshSaved") : t("account.refreshMissing") }}</dd>
        </div>
      </dl>
      <div class="profile-actions">
        <button :disabled="refreshing" @click="refresh">{{ refreshing ? t("account.refreshing") : t("account.refreshToken") }}</button>
        <button class="ghost-danger" @click="logout">{{ t("account.logoutBtn") }}</button>
      </div>
    </div>
        </CardItem>
      </CardBody>
    </CardContainer>

    <div v-else class="panel login">
      <h2>{{ t("account.bindTitle") }}</h2>
      <p>
        {{ t("account.bindLine1") }}<br />
        {{ t("account.bindLine2") }}
      </p>
      <button
        class="primary"
        :disabled="phase === 'waiting' || phase === 'exchanging'"
        @click="beginLogin"
      >
        {{ phase === "waiting" || phase === "exchanging" ? t("account.signingIn") : t("account.signIn") }}
      </button>
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
.acc-3d {
  display: block;
  width: 100%;
  padding: 0;
}

.acc-3d :deep(.acc-3d-body) {
  width: 100%;
  height: auto;
}

.acc-3d :deep(.acc-3d-item) {
  width: 100%;
}

.profile {
  display: flex;
  flex-direction: column;
  gap: 20px;
  width: 100%;
  max-width: 760px;
  margin: 0 auto;
  padding: 26px 28px;
}

.profile-main {
  display: flex;
  align-items: flex-start;
  gap: 18px;
}

.face {
  display: grid;
  place-items: center;
  width: 68px;
  height: 68px;
  flex: none;
  border-radius: 18px;
  font-size: 26px;
  font-weight: 800;
  color: #f4fbff;
  background: linear-gradient(150deg, #3eb1ff 0%, #0072c6 62%, #04487d 100%);
  border: 1px solid rgba(255, 255, 255, 0.22);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4), 0 16px 34px -14px rgba(0, 149, 255, 0.85);
  text-shadow: 0 1px 3px rgba(0, 28, 56, 0.6);
}

.profile-main h2 {
  margin: 6px 0 4px;
  font-size: 22px;
  font-weight: 700;
  letter-spacing: -0.025em;
  line-height: 1.2;
}

.uuid {
  margin: 0;
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 11px;
  letter-spacing: 0.02em;
  color: var(--text-faint);
  word-break: break-all;
}

.facts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 10px;
}

.facts div {
  padding: 12px 15px;
  border-radius: 12px;
  border: 1px solid rgba(126, 160, 199, 0.12);
  background: rgba(255, 255, 255, 0.025);
}

.facts dt {
  margin-bottom: 4px;
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

.profile-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding-top: 4px;
}

/* ── 未登录态 ── */
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
/* ── 账号页：大号身份卡 ── */
.account-card,
.panel.account {
  position: relative;
  overflow: hidden;
}

.avatar {
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.16), 0 18px 40px -18px rgba(0, 149, 255, 0.85);
  animation: ll-glow-pulse 5.5s ease-in-out infinite;
}

</style>
