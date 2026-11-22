/**
 * Language preference: either an explicit choice or "follow the system language".
 *
 * Only explicit choices are stored. Without one, the language is matched from the system
 * (WebView2 reports the Windows display language) on every start, so it follows the system.
 */

/** Languages with a translation folder under `lang/`, keyed by the code i18next uses. */
export const SUPPORTED_LANGUAGES: { [key: string]: string } = {
  en: "English",
  "zh-CN": "简体中文",
  "zh-TW": "繁體中文",
  "ko-KR": "한국어",
  jp: "日本語",
  "fr-FR": "Français",
  bp: "Brazillian Portuguese",
  ge: "Deutsch",
  "es-ES": "Español",
  "it-IT": "Italiano",
};

/** Preference value meaning "use the system language". */
export const SYSTEM_LANGUAGE = "system";

const PREFERENCE_KEY = "gbfr-language";
/** Where i18next's detector cached its pick before; it could not tell a choice from a guess. */
const LEGACY_KEY = "i18nextLng";

/**
 * Maps a BCP 47 tag (e.g. `ja-JP`, `zh-Hant-HK`) to a supported language code. The project's
 * folders use some non-standard codes (`jp`, `ge`, `bp`), hence the explicit table.
 */
export const matchSupportedLanguage = (tag: string): string | null => {
  const lower = tag.toLowerCase();
  const [primary] = lower.split(/[-_]/);

  if (primary === "zh") {
    const traditional = /-(hant|tw|hk|mo)\b/.test(lower);
    return traditional ? "zh-TW" : "zh-CN";
  }

  const byPrimary: { [key: string]: string } = {
    en: "en",
    ja: "jp",
    ko: "ko-KR",
    fr: "fr-FR",
    de: "ge",
    es: "es-ES",
    it: "it-IT",
    pt: "bp",
  };
  return byPrimary[primary] ?? null;
};

/** The first supported language among the system's preferred ones, or English. */
export const systemLanguage = (tags: readonly string[] = navigatorLanguages()): string => {
  for (const tag of tags) {
    const match = matchSupportedLanguage(tag);
    if (match) return match;
  }
  return "en";
};

const navigatorLanguages = (): readonly string[] => {
  if (typeof navigator === "undefined") return [];
  return navigator.languages?.length ? navigator.languages : [navigator.language];
};

const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

/**
 * The saved preference: a supported language code, or `SYSTEM_LANGUAGE`.
 *
 * A language cached by the old detector is kept as a choice unless it is English, which was
 * what the detector fell back to for unmatched system languages.
 */
export const getLanguagePreference = (): string => {
  const store = storage();
  try {
    const saved = store?.getItem(PREFERENCE_KEY);
    if (saved && (saved === SYSTEM_LANGUAGE || saved in SUPPORTED_LANGUAGES)) return saved;

    const legacy = store?.getItem(LEGACY_KEY);
    if (legacy) {
      store?.removeItem(LEGACY_KEY);
      if (legacy in SUPPORTED_LANGUAGES && legacy !== "en") {
        store?.setItem(PREFERENCE_KEY, legacy);
        return legacy;
      }
    }
  } catch {
    // Storage can be unavailable; following the system is the safe default.
  }
  return SYSTEM_LANGUAGE;
};

/** Stores a preference; `SYSTEM_LANGUAGE` clears the explicit choice. */
export const saveLanguagePreference = (preference: string) => {
  try {
    if (preference === SYSTEM_LANGUAGE) {
      storage()?.removeItem(PREFERENCE_KEY);
    } else if (preference in SUPPORTED_LANGUAGES) {
      storage()?.setItem(PREFERENCE_KEY, preference);
    }
  } catch {
    // Not persisted; the language still changes for this session.
  }
};

/** The language code a preference stands for. */
export const resolveLanguage = (preference: string): string =>
  preference in SUPPORTED_LANGUAGES ? preference : systemLanguage();
