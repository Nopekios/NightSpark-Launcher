import { createApp } from "vue";
import App from "./App.vue";
import { i18n } from "./i18n";
import "./styles/global.css";

const app = createApp(App);
app.use(i18n);

// 开发期错误兜底：把运行时异常直接怼到页面上，方便截图定位
if (import.meta.env.DEV) {
  const paint = (title: string, detail: string) => {
    const el = document.createElement("pre");
    el.style.cssText =
      "position:fixed;inset:0;z-index:99999;margin:0;padding:24px;overflow:auto;" +
      "background:#150608;color:#ff8a8a;font:12px/1.6 ui-monospace,Consolas,monospace;white-space:pre-wrap";
    el.textContent = `[${title}]\n${detail}`;
    document.body.appendChild(el);
  };
  window.addEventListener("error", (e) => paint("window.error", String(e.error?.stack || e.message)));
  window.addEventListener("unhandledrejection", (e) =>
    paint("unhandledrejection", String((e.reason as Error)?.stack || e.reason))
  );
  app.config.errorHandler = (err) => paint("vue.error", String((err as Error)?.stack || err));
}

app.mount("#app");

// 关掉 WebView 自带的右键菜单（刷新 / 另存为 / 打印 / 检查元素那一套）。
// WebView2 的这份菜单是浏览器的默认菜单，和启动器功能无关，误点刷新还会把
// 正在跑的界面状态清掉，所以整窗一律拦掉；输入框放行，否则右键粘贴就没了。
document.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement | null;
  const editable = el?.closest("input, textarea, [contenteditable='true']");
  if (editable) return;
  e.preventDefault();
});

// 顺带拦掉浏览器级的快捷键刷新 / 打印 / 保存，避免 F5、Ctrl+R 把前端重载。
window.addEventListener("keydown", (e) => {
  const key = e.key.toLowerCase();
  const browserKey =
    e.key === "F5" ||
    e.key === "F7" ||
    (e.ctrlKey && (key === "r" || key === "p" || key === "s" || key === "u")) ||
    (e.ctrlKey && e.shiftKey && (key === "i" || key === "j" || key === "c"));
  if (browserKey) e.preventDefault();
});
