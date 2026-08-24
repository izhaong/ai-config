import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import enUS from "./locales/en-US.json";
import zhCN from "./locales/zh-CN.json";

export const LOCALE_STORAGE_KEY = "agent-manager.locale";
export const SUPPORTED_LOCALES = ["zh-CN", "en-US"] as const;
export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

function detectLocale(): AppLocale {
  const stored = localStorage.getItem(LOCALE_STORAGE_KEY);
  if (stored === "zh-CN" || stored === "en-US") {
    return stored;
  }
  const nav = navigator.language.toLowerCase();
  if (nav.startsWith("zh")) {
    return "zh-CN";
  }
  return "en-US";
}

function applyDocumentLang(locale: AppLocale) {
  document.documentElement.lang = locale === "zh-CN" ? "zh-CN" : "en";
}

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    "en-US": { translation: enUS },
  },
  lng: detectLocale(),
  fallbackLng: "zh-CN",
  interpolation: { escapeValue: false },
});

applyDocumentLang(i18n.language as AppLocale);

i18n.on("languageChanged", (lng) => {
  if (lng === "zh-CN" || lng === "en-US") {
    localStorage.setItem(LOCALE_STORAGE_KEY, lng);
    applyDocumentLang(lng);
  }
});

export async function setAppLocale(locale: AppLocale): Promise<void> {
  await i18n.changeLanguage(locale);
}

export default i18n;
