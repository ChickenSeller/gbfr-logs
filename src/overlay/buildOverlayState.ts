import { ATTACK_CATEGORIES, ATTACK_CATEGORY_COLORS, attackComposition } from "@/attackCategories";
import { formatCombatLogEntry, partyCharacters } from "@/combatLog";
import { computeSkillBreakdown } from "@/components/useSkillBreakdown";
import { isDebuff, isHighlighted, sortStatuses, statusName } from "@/statuses";
import {
  CharacterType,
  CombatLogEntry,
  ComputedPlayerState,
  ComputedSkillGroup,
  ComputedSkillState,
  EncounterState,
  MeterColumns,
  OverlayDamageTaken,
  OverlayEnemy,
  OverlayLogLine,
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
/** How many combat log lines the in-game window gets. */
export const OVERLAY_COMBAT_LOG_LINES = 60;

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
  settings: OverlaySettings,
  combatLog: CombatLogEntry[] = [],
  trackedEnemies: number[] = []
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
    enemies: overlayEnemies(encounterState, trackedEnemies),
    damageTaken: overlayDamageTaken(encounterState, partyData, players, partyColors, values),
    combatLog: overlayCombatLog(combatLog, encounterState, partyData, partyColors, values),
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

/** The latest combat log lines; streamer mode keeps only the player's own. */
const overlayCombatLog = (
  entries: CombatLogEntry[],
  encounterState: EncounterState,
  partyData: Array<PlayerData | null>,
  partyColors: string[],
  values: OverlaySettingsValues
): OverlayLogLine[] => {
  const context = {
    partyData,
    characters: partyCharacters(encounterState),
    partyColors,
    showDisplayNames: values.showDisplayNames,
  };
  const ownIndex = partyData[0]?.actorIndex;
  return entries
    .map((entry) => formatCombatLogEntry(entry, context))
    .filter((line) => !values.streamerMode || line.actorIndex === ownIndex)
    .slice(-OVERLAY_COMBAT_LOG_LINES)
    .map((line) => ({ time: line.time, text: line.text, color: hexToRgb(line.color) }));
};

/** Party members' damage taken with the meter's names and colours, most first. */
const overlayDamageTaken = (
  encounterState: EncounterState,
  partyData: Array<PlayerData | null>,
  players: ComputedPlayerState[],
  partyColors: string[],
  values: OverlaySettingsValues
): OverlayDamageTaken[] => {
  const taken = encounterState.damageTaken ?? {};
  const statuses = encounterState.statuses ?? {};
  const characters = partyCharacters(encounterState);
  // Members who took damage or got a debuff, most damage first.
  const indices = [...new Set([...Object.keys(taken), ...Object.keys(statuses)].map(Number))].sort(
    (a, b) => (taken[b]?.totalDamage ?? 0) - (taken[a]?.totalDamage ?? 0)
  );

  return indices.flatMap((index, i): OverlayDamageTaken[] => {
    const debuffs = sortStatuses(Object.values(statuses[index] ?? {}))
      .filter((stats) => isDebuff(stats.className))
      .map((stats) => ({
        name: statusName(stats.className),
        count: stats.count,
        totalMs: toCount(stats.totalMs),
        damage: toCount(stats.damageWhileActive ?? 0),
        highlight: isHighlighted(stats.className),
      }));
    const memberTaken = taken[index];
    if (!memberTaken && debuffs.length === 0) return [];

    const partySlotIndex = partyData.findIndex((member) => member?.actorIndex === index);
    if (values.streamerMode && partySlotIndex !== 0) return [];
    const player = players.find((p) => p.index === index) ?? {
      index,
      characterType: memberTaken?.characterType ?? characters.get(index) ?? { Unknown: 0 },
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
        name: translatedPlayerName(partySlotIndex, partyData[partySlotIndex] ?? null, player, values.showDisplayNames),
        color: hexToRgb(resolvePlayerColor(partyColors, partySlotIndex, player.partyIndex) ?? ""),
        totalDamage: toCount(memberTaken?.totalDamage ?? 0),
        hits: toCount(memberTaken?.hits ?? 0),
        maxHit: toCount(memberTaken?.maxHit ?? 0),
        debuffs,
      },
    ];
  });
};

/** One name per enemy type hit in the encounter. */
/** Names for every enemy that took damage, and for the ones the overlay tracks besides. */
const overlayEnemies = (encounterState: EncounterState, trackedEnemies: number[] = []): OverlayEnemy[] => {
  const hashes = new Set<number>(trackedEnemies.map((hash) => hash >>> 0));
  for (const target of Object.values(encounterState.targets ?? {})) {
    if (typeof target.rawTargetType === "number") hashes.add(target.rawTargetType >>> 0);
  }
  return [...hashes].map((typeHash) => ({ typeHash, name: translateEnemyTypeId(typeHash) }));
};
