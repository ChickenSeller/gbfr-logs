import { describe, expect, it } from "vitest";

import { matchSupportedLanguage, resolveLanguage, systemLanguage } from "./language";

describe("matchSupportedLanguage", () => {
  it.each([
    ["en-US", "en"],
    ["zh-CN", "zh-CN"],
    ["zh-SG", "zh-CN"],
    ["zh-Hans", "zh-CN"],
    ["zh", "zh-CN"],
    ["zh-TW", "zh-TW"],
    ["zh-HK", "zh-TW"],
    ["zh-Hant-MO", "zh-TW"],
    ["ja-JP", "jp"],
    ["ko-KR", "ko-KR"],
    ["de-DE", "ge"],
    ["pt-BR", "bp"],
    ["fr-CA", "fr-FR"],
    ["es-MX", "es-ES"],
    ["it-IT", "it-IT"],
  ])("maps %s to %s", (tag, expected) => {
    expect(matchSupportedLanguage(tag)).toBe(expected);
  });

  it("has no match for unsupported languages", () => {
    expect(matchSupportedLanguage("ru-RU")).toBeNull();
  });
});

describe("systemLanguage", () => {
  it("takes the first supported system language", () => {
    expect(systemLanguage(["ru-RU", "ja-JP", "en-US"])).toBe("jp");
  });

  it("falls back to English", () => {
    expect(systemLanguage(["ru-RU"])).toBe("en");
    expect(systemLanguage([])).toBe("en");
  });
});

describe("resolveLanguage", () => {
  it("keeps an explicit choice", () => {
    expect(resolveLanguage("ko-KR")).toBe("ko-KR");
  });
});
