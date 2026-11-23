import { createI18n } from "vue-i18n";
import zhCN from "./locales/zh-CN";
import enUS from "./locales/en-US";

export type Locale = "zh-CN" | "en-US";

export const SUPPORTED_LOCALES: { value: Locale; label: string }[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en-US", label: "English" },
];

export function normalizeLocale(value: string | null | undefined): Locale | null {
  if (!value) return null;
  const v = value.trim().toLowerCase();
  if (v.startsWith("zh")) return "zh-CN";
  if (v.startsWith("en")) return "en-US";
  return null;
}

function detectSystemLocale(): Locale {
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    const hit = normalizeLocale(tag);
    if (hit) return hit;
  }
  return "zh-CN";
}

export const i18n = createI18n({
  legacy: false,
  locale: detectSystemLocale(),
  fallbackLocale: "zh-CN",
  messages: { "zh-CN": zhCN, "en-US": enUS },
});

// 首屏就按检测到的语言标记 lang，避免切换前 html lang 与实际渲染语言不一致
document.documentElement.setAttribute("lang", i18n.global.locale.value as string);

export function currentLocale(): Locale {
  return i18n.global.locale.value as Locale;
}

export function setLocale(locale: Locale) {
  i18n.global.locale.value = locale;
  document.documentElement.setAttribute("lang", locale);
}

export function t(key: string, named?: Record<string, unknown>): string {
  const translate = i18n.global.t as (k: string, n?: Record<string, unknown>) => string;
  return named ? translate(key, named) : translate(key);
}
