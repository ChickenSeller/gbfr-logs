import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import resourcesToBackend from "i18next-resources-to-backend";
import { initReactI18next } from "react-i18next";

import { readTextFile } from "@tauri-apps/api/fs";
import { resolveResource } from "@tauri-apps/api/path";

import { getLanguagePreference, resolveLanguage, saveLanguagePreference } from "./language";

const loadLanguageFromPath = async (language: string, namespace: string) => {
  const resourcePath = await resolveResource(`lang/${language}/${namespace}.json`);
  return JSON.parse(await readTextFile(resourcePath));
};

/** Picks the saved choice, or the system language when there is none. */
const detector = new LanguageDetector();
detector.addDetector({
  name: "preference",
  lookup: () => resolveLanguage(getLanguagePreference()),
});

/** Changes the language and remembers the choice (`SYSTEM_LANGUAGE` follows the system). */
export const changeLanguagePreference = (preference: string) => {
  saveLanguagePreference(preference);
  return i18n.changeLanguage(resolveLanguage(preference));
};

i18n
  .use(detector)
  .use(initReactI18next)
  .use(
    resourcesToBackend((language, namespace, callback) => {
      loadLanguageFromPath(language, namespace)
        .then((res) => callback(null, res))
        .catch((error) => callback(error, null));
    })
  )
  .init({
    ns: ["ui", "characters", "items", "overmasteries", "sigils", "traits", "weapons", "quests", "enemies"],
    defaultNS: "ui",
    // Only explicit choices are stored (see ./language), so nothing is cached here.
    detection: { order: ["preference"], caches: [] },
    fallbackLng: {
      default: ["en"],
      "zh-TW": ["zh-CN", "en"],
    },
    interpolation: {
      escapeValue: false,
    },
    react: {
      bindI18n: "languageChanged loaded",
      bindI18nStore: "added",
    },
  });

declare global {
  interface Window {
    /* eslint-disable */
    i18n: any;
  }
}

window.i18n = i18n;

export default i18n;
