import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { de } from "./de";
import { en } from "./en";
import { es } from "./es";
import { fr } from "./fr";
import { id } from "./id";
import { it } from "./it";
import { ja } from "./ja";
import { ko } from "./ko";
import { resolveLang, type Lang } from "./languages";
import { pl } from "./pl";
import { pt } from "./pt";
import { ru } from "./ru";
import { tr } from "./tr";
import { uk } from "./uk";
import { zh } from "./zh";

export { resolveLang, type Lang } from "./languages";

export const TRANSLATIONS = { en, tr, de, es, fr, it, pt, pl, ru, uk, id, ja, ko, zh } as const;

function cachedLang(): string | undefined {
  try {
    return localStorage.getItem("reclaim:lang") ?? undefined;
  } catch {
    return undefined;
  }
}

const resources = Object.fromEntries(Object.entries(TRANSLATIONS).map(([code, translation]) => [code, { translation }]));

// Missing keys in a translation fall back to English, so a new feature never shows raw keys.
void i18n.use(initReactI18next).init({
  resources,
  lng: resolveLang(cachedLang()),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export function setLanguage(pref: string): Lang {
  const lang = resolveLang(pref);
  try {
    localStorage.setItem("reclaim:lang", pref);
  } catch {
    // Storage can be unavailable; the language still applies for this session.
  }
  // The lang attribute also lets WebView2 pick the right CJK glyph forms.
  document.documentElement.lang = lang;
  void i18n.changeLanguage(lang);
  return lang;
}

export default i18n;
