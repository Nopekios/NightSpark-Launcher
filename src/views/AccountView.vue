<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errText, type Account } from "../api";

const props = defineProps<{ account: Account | null }>();

const emit = defineEmits<{
  (e: "changed", account: Account | null): void;
  (e: "notify", text: string, tone: "ok" | "err"): void;
}>();

type Phase = "idle" | "waiting" | "exchanging" | "error";

const phase = ref<Phase>("idle");
const progress = ref("");
const errorText = ref("");
const refreshing = ref(false);
let unlisteners: UnlistenFn[] = [];

const expireText = computed(() => {
  if (!props.account || !props.account.expiresAt) return "未知";
  const left = props.account.expiresAt - Math.floor(Date.now() / 1000);
  if (left <= 0) return "已过期（需要刷新）";
  const minutes = Math.floor(left / 60);
  if (minutes < 60) return `${minutes} 分钟后过期`;
  return `${Math.floor(minutes / 60)} 小时后过期`;
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
    emit("notify", `已登录：${account.name}`, "ok");
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
  progress.value = "正在打开微软登录窗口…";

  unlisteners.push(
    await listen<string>("login-progress", (event) => {
      progress.value = event.payload;
    })
  );
  unlisteners.push(
    await listen<string>("msa-code", (event) => {
      progress.value = "已拿到授权码，正在换取令牌…";
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
    emit("notify", "令牌已刷新", "ok");
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
    emit("notify", "已退出账号", "ok");
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
        <h1>账号</h1>
        <p>只支持微软正版登录，走官方 XBL 链路拿 Minecraft 令牌。</p>
      </div>
      <span v-if="account" class="tag">已连接</span>
      <span v-else class="tag muted">未登录</span>
    </header>

    <div v-if="account" class="panel profile">
      <div class="profile-main">
        <div class="face">{{ account.name.slice(0, 1).toUpperCase() }}</div>
        <div>
          <h2>{{ account.name }}</h2>
          <p class="uuid">{{ account.uuid }}</p>
        </div>
      </div>
      <dl class="facts">
        <div>
          <dt>令牌状态</dt>
          <dd>{{ expireText }}</dd>
        </div>
        <div>
          <dt>刷新令牌</dt>
          <dd>{{ account.hasRefreshToken ? "已保存（可免登录续期）" : "未下发" }}</dd>
        </div>
      </dl>
      <div class="profile-actions">
        <button :disabled="refreshing" @click="refresh">{{ refreshing ? "刷新中…" : "刷新令牌" }}</button>
        <button class="ghost-danger" @click="logout">退出登录</button>
      </div>
    </div>

    <div v-else class="panel login">
      <h2>绑定 Microsoft 账号</h2>
      <p>
        点击后会弹出微软官方登录窗口，登录完成后窗口自动关闭。<br />
        授权码留在本地换取令牌，密码不会经过启动器。
      </p>
      <button
        class="primary"
        :disabled="phase === 'waiting' || phase === 'exchanging'"
        @click="beginLogin"
      >
        {{ phase === "waiting" || phase === "exchanging" ? "登录中…" : "使用 Microsoft 登录" }}
      </button>
      <p v-if="progress" class="progress">{{ progress }}</p>
      <p v-if="phase === 'error'" class="error">{{ errorText }}</p>
      <ul class="tips">
        <li>需要账号已购买 Minecraft: Java 版，否则最后一步查档案会 404。</li>
        <li>报「还没有 Xbox 档案」时，先去 xbox.com 建一个角色再回来。</li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.profile {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.profile-main {
  display: flex;
  align-items: center;
  gap: 16px;
}

.face {
  width: 58px;
  height: 58px;
  border-radius: 16px;
  display: grid;
  place-items: center;
  font-size: 24px;
  font-weight: 700;
  color: #06120a;
  background: linear-gradient(135deg, var(--grass), var(--grass-deep));
  box-shadow: 0 10px 26px rgba(90, 216, 111, 0.25);
}

.profile-main h2 {
  margin: 0 0 4px;
  font-size: 19px;
}

.uuid {
  margin: 0;
  font-size: 12px;
  color: var(--text-faint);
  letter-spacing: 0.5px;
}

.facts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: 12px;
  margin: 0;
}

.facts div {
  padding: 12px 14px;
  border-radius: var(--radius-md);
  border: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.03);
}

.facts dt {
  font-size: 11px;
  color: var(--text-faint);
  margin-bottom: 5px;
}

.facts dd {
  margin: 0;
  font-size: 13px;
}

.profile-actions {
  display: flex;
  gap: 10px;
}

.login {
  max-width: 620px;
}

.login h2 {
  margin: 0 0 10px;
  font-size: 17px;
}

.login p {
  margin: 0 0 18px;
  font-size: 13px;
  line-height: 1.75;
  color: var(--text-dim);
}

.login .progress {
  margin: 16px 0 0;
  color: var(--grass);
}

.login .error {
  margin: 16px 0 0;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  border: 1px solid rgba(239, 90, 90, 0.35);
  background: rgba(239, 90, 90, 0.1);
  color: #ffb4b4;
  word-break: break-word;
}

.tips {
  margin: 20px 0 0;
  padding-left: 18px;
  font-size: 12px;
  line-height: 1.9;
  color: var(--text-faint);
}
</style>
