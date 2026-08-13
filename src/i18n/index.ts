// i18n 初始化。
//
// 设计要点：
// - UI 界面语言与 ASR「录入语言」(AppConfig.local_language) 物理隔离：
//   UI 语言只存在 localStorage，纯前端，不影响任何后端配置。
// - 默认中文（zh）；支持 zh / en。
// - 切换时持久化 + 同步 <html lang>。
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "./locales/zh.json";
import en from "./locales/en.json";

/** localStorage key：界面语言偏好（与后端 AppConfig 完全无关）。 */
export const UI_LANG_KEY = "openime.ui_lang";

export type UiLang = "zh" | "en";

/** 探测初始语言：localStorage > 默认 zh。 */
function detectLanguage(): UiLang {
  try {
    const saved = localStorage.getItem(UI_LANG_KEY);
    if (saved === "zh" || saved === "en") return saved;
  } catch {
    /* localStorage 不可用（隐私模式等），回退默认 */
  }
  return "zh";
}

function applyHtmlLang(lng: string) {
  document.documentElement.lang = lng === "en" ? "en" : "zh-Hans";
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

// 语言切换：持久化 + 同步 <html lang>。
i18n.on("languageChanged", (lng) => {
  applyHtmlLang(lng);
  try {
    localStorage.setItem(UI_LANG_KEY, lng);
  } catch {
    /* ignore */
  }
});

applyHtmlLang(initial);

export default i18n;
