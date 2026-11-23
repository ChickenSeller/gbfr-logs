import i18n from "i18next";
import { beforeAll, describe, expect, it, vi } from "vitest";

import { combatLogText, formatCombatLogEntry, formatTime, kindOf } from "./combatLog";
import { appendCombatLogEntry } from "./overlay/OverlayBridge";
import { CombatLogEntry } from "./types";

vi.mock("@/assets/skill-groups", () => ({ default: {} }));

beforeAll(async () => {
  await i18n.init({
    lng: "en",
    defaultNS: "ui",
    ns: ["ui"],
    interpolation: { escapeValue: false },
    resources: {
      en: {
        ui: {
          ui: {
            "combat-log": {
              damage: "{{source}} › {{skill}} → {{target}}  {{damage}}",
              "damage-taken": "{{source}} → {{target}}  {{damage}}",
              death: "{{actor}} was knocked out",
            },
          },
          skills: { Pl0000: { "100": "Attack 1" } },
          enemies: { "unknown-type": "Enemy {{id}}" },
        },
      },
    },
  });
});

const context = {
  partyData: [null, null, null, null],
  characters: new Map([[1, "Pl0000"]]),
  partyColors: ["#FF5630", "#FFAB00", "#36B37E", "#00B8D9"],
  showDisplayNames: false,
};

describe("formatTime", () => {
  it("formats minutes and tenths", () => {
    expect(formatTime(0)).toBe("00:00.0");
    expect(formatTime(1_500)).toBe("00:01.5");
    expect(formatTime(75_250)).toBe("01:15.3");
  });
});

describe("formatCombatLogEntry", () => {
  it("writes damage dealt with skill, target and amount", () => {
    const entry: CombatLogEntry = {
      kind: "damage",
      time: 1_500,
      sourceIndex: 1,
      sourceType: "Pl0000",
      childType: "Pl0000",
      targetType: 0x12345678,
      action: { Normal: 100 },
      damage: 12_345,
    };
    const line = formatCombatLogEntry(entry, context);
    expect(line.time).toBe("00:01.5");
    expect(line.kind).toBe("damage");
    expect(line.actorIndex).toBe(1);
    expect(line.text).toContain("Attack 1");
    expect(line.text).toContain("12,345");
  });

  it("groups SBA entries and writes knockouts", () => {
    expect(kindOf({ kind: "sbaChain", time: 0, actorIndex: 1 })).toBe("sba");
    const line = formatCombatLogEntry({ kind: "death", time: 0, actorIndex: 1 }, context);
    expect(line.kind).toBe("death");
    expect(line.text).toContain("was knocked out");
    expect(combatLogText([line])).toBe(`[00:00.0] ${line.text}`);
  });
});

describe("appendCombatLogEntry", () => {
  const death = (time: number): CombatLogEntry => ({ kind: "death", time, actorIndex: 1 });

  it("appends and starts over when the times restart", () => {
    const log = appendCombatLogEntry(appendCombatLogEntry([], death(5_000)), death(6_000));
    expect(log.map((e) => e.time)).toEqual([5_000, 6_000]);
    expect(appendCombatLogEntry(log, death(10)).map((e) => e.time)).toEqual([10]);
  });

  it("keeps a bounded number of entries", () => {
    let log: CombatLogEntry[] = [];
    for (let i = 0; i < 500; i++) log = appendCombatLogEntry(log, death(i * 10));
    expect(log.length).toBe(120);
    expect(log[log.length - 1].time).toBe(4_990);
  });
});
