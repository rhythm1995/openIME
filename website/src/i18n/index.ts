// 站点 i18n 初始化（与 app 的 src/i18n 约定一致，storage key 独立）。
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "./locales/zh.json";
import en from "./locales/en.json";

export const SITE_LANG_KEY = "openime.site_lang";
export type SiteLang = "zh" | "en";

function detectLanguage(): SiteLang {
  try {
    const saved = localStorage.getItem(SITE_LANG_KEY);
    if (saved === "zh" || saved === "en") return saved;
  } catch {
    /* localStorage 不可用，回退默认 */
  }
  return "zh";
}

function applyDocument(lng: string) {
  document.documentElement.lang = lng === "en" ? "en" : "zh-Hans";
  document.title = i18n.t("meta.title");
}

const initial = detectLanguage();

i18n.use(initReactI18next).init({
  resources: {
    zh: { translation: zh },
    en: { translation: en },
  },
  lng: initial,
  fallbackLng: "zh",
  interpolation: { escapeValue: false },
});

i18n.on("languageChanged", (lng) => {
  applyDocument(lng);
  try {
    localStorage.setItem(SITE_LANG_KEY, lng);
  } catch {
    /* ignore */
  }
});

applyDocument(initial);

export default i18n;
