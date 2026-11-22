import { ATTACK_CATEGORIES, ATTACK_CATEGORY_COLORS, attackComposition } from "@/attackCategories";
import { computeSkillBreakdown } from "@/components/useSkillBreakdown";
import {
  CharacterType,
  ComputedPlayerState,
  ComputedSkillGroup,
  ComputedSkillState,
  EncounterState,
  MeterColumns,
  OverlayDamageTaken,
  OverlayEnemy,
  OverlayPlayer,
  OverlaySettings,
  OverlaySettingsValues,
  OverlaySkill,
  OverlayState,
  PlayerData,
} from "@/types";
import {
  formatInPartyOrder,
  getSkillName,
  hasOwnSkillName,
  resolvePlayerColor,
  translateEnemyTypeId,
  translatedPlayerName,
} from "@/utils";
import { t } from "i18next";

const FALLBACK_COLOR = 0xffffff;

/** Converts `#RRGGBB` to `0xRRGGBB`. */
export const hexToRgb = (hex: string): number => {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  return match ? parseInt(match[1], 16) : FALLBACK_COLOR;
};

/** Converts `0xRRGGBB` to `#RRGGBB`, the format the settings page stores. */
export const rgbToHex = (rgb: number): string =>
  `#${(Math.max(0, Math.min(0xffffff, Math.round(rgb))) >>> 0).toString(16).padStart(6, "0").toUpperCase()}`;

/** The settings page's meter options, as stored in `useMeterSettingsStore`. */
export type MeterSettings = {
  color_1: string;
  color_2: string;
  color_3: string;
  color_4: string;
  transparency: number;
  show_display_names: boolean;
  streamer_mode: boolean;
  show_full_values: boolean;
  use_condensed_skills: boolean;
  open_log_on_save: boolean;
  overlay_columns: MeterColumns[];
};

/** Columns that can be shown after the player name. */
export const VALUE_COLUMNS = Object.values(MeterColumns).filter((column) => column !== MeterColumns.Name);

/**
 * Everything the in-game settings menu shows: current values, choices and the settings page's
 * translated labels.
 */
export const buildOverlaySettings = (
  meter: MeterSettings,
  language: string,
  languages: Array<{ value: string; label: string }>,
  t: (key: string) => string
): OverlaySettings => ({
  values: {
    language,
    partyColors: [meter.color_1, meter.color_2, meter.color_3, meter.color_4].map(hexToRgb),
    transparency: meter.transparency,
    showDisplayNames: meter.show_display_names,
    streamerMode: meter.streamer_mode,
    showFullValues: meter.show_full_values,
    useCondensedSkills: meter.use_condensed_skills,
    openLogOnSave: meter.open_log_on_save,
    columns: meter.overlay_columns,
  },
  languages,
  columns: VALUE_COLUMNS.map((column) => ({
    column,
    label: t(`ui.meter-columns.${column}`),
    description: t(`ui.meter-columns.${column}-description`),
  })),
  labels: {
    title: t("ui.meter-settings"),
    language: t("ui.language"),
    partyColors: [1, 2, 3, 4].map((slot) => t(`ui.player-${slot}-color`)),
    transparency: t("ui.meter-transparency"),
    showDisplayNames: t("ui.show-player-names"),
    streamerMode: t("ui.streamer-mode"),
    streamerModeDescription: t("ui.streamer-mode-description"),
    showFullValues: t("ui.show-full-values"),
    showFullValuesDescription: t("ui.show-full-values-description"),
    useCondensedSkills: t("ui.use-condensed-skills"),
    useCondensedSkillsDescription: t("ui.use-condensed-skills-description"),
    openLogOnSave: t("ui.open-log-on-save"),
    openLogOnSaveDescription: t("ui.open-log-on-save-description"),
    columns: "Customize Overlay Meter Columns",
  },
});

/**
 * Turns values edited in game back into store settings. The overlay is another process, so
 * everything is validated: unknown or duplicate columns are dropped and numbers clamped.
 */
export const meterSettingsFromOverlay = (values: OverlaySettingsValues): Partial<MeterSettings> => {
  const columns = values.columns.filter(
    (column, index) => VALUE_COLUMNS.includes(column) && values.columns.indexOf(column) === index
  );
  const colors = values.partyColors.slice(0, 4).map(rgbToHex);
  const settings: Partial<MeterSettings> = {
    transparency: Math.min(Math.max(Number(values.transparency) || 0, 0), 1),
    show_display_names: Boolean(values.showDisplayNames),
    streamer_mode: Boolean(values.streamerMode),
    show_full_values: Boolean(values.showFullValues),
    use_condensed_skills: Boolean(values.useCondensedSkills),
    open_log_on_save: Boolean(values.openLogOnSave),
    overlay_columns: columns,
  };
  const keys = ["color_1", "color_2", "color_3", "color_4"] as const;
  colors.forEach((color, slot) => {
    settings[keys[slot]] = color;
  });
  return settings;
};

/** Rust receives these as unsigned integers. */
const toCount = (value: number | null | undefined): number => Math.max(0, Math.round(value || 0));

const toOverlaySkill = (characterType: CharacterType, skill: ComputedSkillState | ComputedSkillGroup): OverlaySkill => {
  const children = "skills" in skill && skill.skills ? skill.skills : [];

  return {
    name: getSkillName(characterType, skill),
    hits: toCount(skill.hits),
    totalDamage: toCount(skill.totalDamage),
    minDamage: skill.minDamage ? toCount(skill.minDamage) : null,
    maxDamage: skill.maxDamage ? toCount(skill.maxDamage) : null,
    percentage: Number.isFinite(skill.percentage) ? skill.percentage : 0,
    children: children.map((child) => toOverlaySkill(characterType, child)),
  };
};

/**
 * Builds the snapshot the in-game meter draws: the desktop meter's names, colours, columns,
 * streamer-mode filter and skill breakdowns. Players stay in party order; the overlay sorts.
 */
export const buildOverlayState = (
  encounterState: EncounterState,
  partyData: Array<PlayerData | null>,
  settings: OverlaySettings
): OverlayState => {
  const { values } = settings;
  const partyColors = values.partyColors.map(rgbToHex);
  const totalDamage = encounterState.totalDamage;
  const players: ComputedPlayerState[] = formatInPartyOrder(encounterState.party).map((player) => ({
    ...player,
    percentage: totalDamage > 0 ? (player.totalDamage / totalDamage) * 100 : 0,
  }));

  const rows = players.flatMap((player): OverlayPlayer[] => {
    const partySlotIndex = partyData.findIndex((partyMember) => partyMember?.actorIndex === player.index);
    if (values.streamerMode && partySlotIndex !== 0) return [];

    const color = resolvePlayerColor(partyColors, partySlotIndex, player.partyIndex);
    return [
      {
        index: toCount(player.index),
        partyOrder: toCount(player.partyIndex),
        name: translatedPlayerName(partySlotIndex, partyData[partySlotIndex] ?? null, player, values.showDisplayNames),
        color: hexToRgb(color ?? ""),
        totalDamage: toCount(player.totalDamage),
        dps: player.dps || 0,
        percentage: player.percentage,
        sba: player.sba || 0,
        totalStunValue: player.totalStunValue || 0,
        stunPerSecond: player.stunPerSecond || 0,
        skills: computeSkillBreakdown(player, values.useCondensedSkills).map((skill) =>
          toOverlaySkill(player.characterType, skill)
        ),
        attackDamage: ((composition) => ATTACK_CATEGORIES.map((category) => toCount(composition[category])))(
          attackComposition(player, hasOwnSkillName)
        ),
      },
    ];
  });

  return {
    status: encounterState.status,
    startTime: Math.round(encounterState.startTime),
    endTime: Math.round(encounterState.endTime),
    totalDamage: toCount(totalDamage),
    dps: encounterState.dps || 0,
    settings,
    players: rows,
    enemies: overlayEnemies(encounterState),
    damageTaken: overlayDamageTaken(encounterState, partyData, players, partyColors, values),
    damageTakenLabels: {
      title: t("ui.logs.damage-taken"),
      hits: t("ui.damage-taken.hits"),
      maxHit: t("ui.damage-taken.max-hit"),
    },
    attackTypes: ATTACK_CATEGORIES.map((category) => ({
      label: t(`ui.attack-types.${category}`),
      color: hexToRgb(ATTACK_CATEGORY_COLORS[category]),
    })),
  };
};

/** Party members' damage taken with the meter's names and colours, most first. */
const overlayDamageTaken = (
  encounterState: EncounterState,
  partyData: Array<PlayerData | null>,
  players: ComputedPlayerState[],
  partyColors: string[],
  values: OverlaySettingsValues
): OverlayDamageTaken[] =>
  Object.values(encounterState.damageTaken ?? {})
    .sort((a, b) => b.totalDamage - a.totalDamage)
    .flatMap((taken, i): OverlayDamageTaken[] => {
      const partySlotIndex = partyData.findIndex((member) => member?.actorIndex === taken.index);
      if (values.streamerMode && partySlotIndex !== 0) return [];
      const player = players.find((p) => p.index === taken.index) ?? {
        index: taken.index,
        characterType: taken.characterType,
        totalDamage: 0,
        dps: 0,
        sba: 0,
        totalStunValue: 0,
        stunPerSecond: 0,
        lastDamageTime: 0,
        skillBreakdown: [],
        partyIndex: i,
        percentage: 0,
      };
      return [
        {
          name: translatedPlayerName(
            partySlotIndex,
            partyData[partySlotIndex] ?? null,
            player,
            values.showDisplayNames
          ),
          color: hexToRgb(resolvePlayerColor(partyColors, partySlotIndex, player.partyIndex) ?? ""),
          totalDamage: toCount(taken.totalDamage),
          hits: toCount(taken.hits),
          maxHit: toCount(taken.maxHit),
        },
      ];
    });

/** One name per enemy type hit in the encounter. */
const overlayEnemies = (encounterState: EncounterState): OverlayEnemy[] => {
  const hashes = new Set<number>();
  for (const target of Object.values(encounterState.targets ?? {})) {
    if (typeof target.rawTargetType === "number") hashes.add(target.rawTargetType >>> 0);
  }
  return [...hashes].map((typeHash) => ({ typeHash, name: translateEnemyTypeId(typeHash) }));
};
