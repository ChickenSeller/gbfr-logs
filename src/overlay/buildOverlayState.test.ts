import { describe, expect, it, vi } from "vitest";

import { EncounterState, MeterColumns, PlayerData, PlayerState } from "@/types";
import {
  MeterSettings,
  buildOverlaySettings,
  buildOverlayState,
  hexToRgb,
  meterSettingsFromOverlay,
  rgbToHex,
} from "./buildOverlayState";

// Skill groups are loaded from the app resources through Tauri, which tests do not have.
vi.mock("@/assets/skill-groups", () => ({ default: {} }));

const player = (index: number, characterType: string, totalDamage: number, dps: number): PlayerState =>
  ({
    index,
    characterType,
    totalDamage,
    dps,
    sba: 500,
    totalStunValue: 12.5,
    stunPerSecond: 0.4,
    lastDamageTime: 0,
    skillBreakdown: [
      {
        actionType: { Normal: 100 },
        childCharacterType: characterType,
        hits: 3,
        minDamage: 10.4,
        maxDamage: null,
        totalDamage,
        totalStunValue: 0,
        maxStunValue: 0,
        damageDetails: null,
      },
    ],
  }) as unknown as PlayerState;

const encounter = (party: PlayerState[]): EncounterState => ({
  totalDamage: party.reduce((sum, p) => sum + p.totalDamage, 0),
  dps: 100,
  startTime: 1000,
  endTime: 31000,
  party: Object.fromEntries(party.map((p) => [String(p.index), p])),
  status: "InProgress",
  targets: {},
});

const slot = (actorIndex: number, displayName: string) => ({ actorIndex, displayName }) as unknown as PlayerData;

const meter: MeterSettings = {
  color_1: "#FF5630",
  color_2: "#FFAB00",
  color_3: "#36B37E",
  color_4: "#00B8D9",
  transparency: 0.2,
  show_display_names: true,
  streamer_mode: false,
  show_full_values: false,
  use_condensed_skills: false,
  open_log_on_save: true,
  overlay_columns: [MeterColumns.TotalDamage, MeterColumns.DPS],
};

const languages = [
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
];
const translate = (key: string) => `t:${key}`;
const settings = buildOverlaySettings(meter, "zh-CN", languages, translate);
const withValues = (values: Partial<typeof settings.values>) => ({
  ...settings,
  values: { ...settings.values, ...values },
});

describe("hexToRgb", () => {
  it("parses #RRGGBB", () => {
    expect(hexToRgb("#FF5630")).toBe(0xff5630);
    expect(hexToRgb("00b8d9")).toBe(0x00b8d9);
  });

  it("falls back to white for anything else", () => {
    expect(hexToRgb("rgba(1,2,3,1)")).toBe(0xffffff);
    expect(hexToRgb("")).toBe(0xffffff);
  });

  it("round-trips with rgbToHex", () => {
    expect(rgbToHex(0xff5630)).toBe("#FF5630");
    expect(rgbToHex(0x0000d9)).toBe("#0000D9");
    expect(hexToRgb(rgbToHex(0x36b37e))).toBe(0x36b37e);
  });
});

describe("buildOverlaySettings", () => {
  it("carries the settings page values, choices and translated labels", () => {
    expect(settings.values).toEqual({
      language: "zh-CN",
      partyColors: [0xff5630, 0xffab00, 0x36b37e, 0x00b8d9],
      transparency: 0.2,
      showDisplayNames: true,
      streamerMode: false,
      showFullValues: false,
      useCondensedSkills: false,
      openLogOnSave: true,
      columns: [MeterColumns.TotalDamage, MeterColumns.DPS],
    });
    expect(settings.languages).toEqual(languages);
    expect(settings.columns.map((info) => info.column)).not.toContain(MeterColumns.Name);
    expect(settings.columns[0].label).toBe(`t:ui.meter-columns.${settings.columns[0].column}`);
    expect(settings.labels.partyColors).toEqual([1, 2, 3, 4].map((slot) => `t:ui.player-${slot}-color`));
  });
});

describe("meterSettingsFromOverlay", () => {
  it("maps in-game edits back to store settings", () => {
    const result = meterSettingsFromOverlay({
      ...settings.values,
      partyColors: [0x112233, 0xffab00, 0x36b37e, 0x00b8d9],
      transparency: 0.5,
      streamerMode: true,
      columns: [MeterColumns.DPS, MeterColumns.SBA],
    });

    expect(result).toMatchObject({
      color_1: "#112233",
      transparency: 0.5,
      streamer_mode: true,
      overlay_columns: [MeterColumns.DPS, MeterColumns.SBA],
    });
  });

  it("drops unknown or repeated columns and clamps numbers", () => {
    const result = meterSettingsFromOverlay({
      ...settings.values,
      transparency: 7,
      columns: [MeterColumns.DPS, MeterColumns.Name, "bogus" as MeterColumns, MeterColumns.DPS],
    });

    expect(result.overlay_columns).toEqual([MeterColumns.DPS]);
    expect(result.transparency).toBe(1);
  });
});

describe("buildOverlayState", () => {
  const state = encounter([player(0, "Pl0000", 300, 10), player(1, "Pl0100", 700, 30)]);
  const partyData = [slot(0, "LLK"), slot(1, "古兰"), null, null];

  it("keeps party order and party slot colours", () => {
    const result = buildOverlayState(state, partyData, settings);

    expect(result.players.map((p) => p.totalDamage)).toEqual([300, 700]);
    expect(result.players.map((p) => p.partyOrder)).toEqual([0, 1]);
    expect(result.players[0].color).toBe(0xff5630);
    expect(result.players[1].color).toBe(0xffab00);
    expect(result.players[1].percentage).toBeCloseTo(70);
    expect(result.players[1].name).toContain("古兰");
  });

  it("uses the colours from the settings", () => {
    const result = buildOverlayState(state, partyData, withValues({ partyColors: [0x010203, 0x040506, 0, 0] }));

    expect(result.players.map((p) => p.color)).toEqual([0x010203, 0x040506]);
    expect(result.settings.values.partyColors[0]).toBe(0x010203);
  });

  it("sends skills with integer counts", () => {
    const [first] = buildOverlayState(state, partyData, settings).players;

    expect(first.skills).toHaveLength(1);
    expect(first.skills[0]).toMatchObject({ hits: 3, totalDamage: 300, minDamage: 10, maxDamage: null });
    expect(first.skills[0].percentage).toBeCloseTo(100);
    expect(first.sba).toBe(500);
  });

  it("only keeps the first party slot in streamer mode", () => {
    const result = buildOverlayState(state, partyData, withValues({ streamerMode: true }));

    expect(result.players).toHaveLength(1);
    expect(result.players[0].name).toContain("LLK");
  });

  it("reports 0% instead of NaN before any damage", () => {
    const empty = encounter([player(0, "Pl0000", 0, 0)]);
    const result = buildOverlayState(empty, partyData, settings);

    expect(result.players[0].percentage).toBe(0);
  });
});
