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
