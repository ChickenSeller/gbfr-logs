/**
 * Turns combat log entries (see `parser::v1::combat_log`) into lines of text in the selected
 * language. Used live by the in-game overlay and for the "Combat Log" tab of saved logs.
 */
import { t } from "i18next";

import { statusName } from "./statuses";
import { CharacterType, CombatLogEntry, ComputedPlayerState, EncounterState, PlayerData, SkillState } from "./types";
import {
  getSkillName,
  resolvePlayerColor,
  translateCharacterType,
  translateEnemyTypeId,
  translatedPlayerName,
} from "./utils";

export const COMBAT_LOG_KINDS = ["damage", "damageTaken", "status", "partBreak", "sba", "death"] as const;
export type CombatLogKind = (typeof COMBAT_LOG_KINDS)[number];

export type CombatLogLine = {
  /** `mm:ss.t` since the start of the encounter */
  time: string;
  text: string;
  kind: CombatLogKind;
  /** Colour of the party member the line is about, `#rrggbb` */
  color: string;
  /** Actor index of that party member */
  actorIndex: number;
};

export type CombatLogContext = {
  partyData: Array<PlayerData | null>;
  /** Character type of each party member by actor index, for entries without one. */
  characters: Map<number, CharacterType>;
  partyColors: string[];
  showDisplayNames: boolean;
};

export const kindOf = (entry: CombatLogEntry): CombatLogKind => {
  switch (entry.kind) {
    case "damage":
      return "damage";
    case "damageTaken":
      return "damageTaken";
    case "death":
      return "death";
    case "statusApplied":
    case "statusRemoved":
      return "status";
    case "partBroken":
      return "partBreak";
    default:
      return "sba";
  }
};

export const formatTime = (ms: number): string => {
  const total = Math.max(0, ms) / 1000;
  const minutes = Math.floor(total / 60);
  const seconds = total - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${seconds.toFixed(1).padStart(4, "0")}`;
};

const formatNumber = (value: number) => value.toLocaleString();

const playerOf = (index: number, characterType: CharacterType): ComputedPlayerState => ({
  index,
  characterType,
  totalDamage: 0,
  dps: 0,
  sba: 0,
  totalStunValue: 0,
  stunPerSecond: 0,
  lastDamageTime: 0,
  skillBreakdown: [],
  partyIndex: 0,
  percentage: 0,
});

/** Name and colour of a party member, as the meter shows them. */
export const combatLogMember = (context: CombatLogContext, index: number, characterType?: CharacterType) => {
  const slot = context.partyData.findIndex((data) => data?.actorIndex === index);
  // Members who neither dealt nor took damage are only known from the party loadouts.
  const type = characterType ??
    context.characters.get(index) ??
    context.partyData[slot]?.characterType ?? { Unknown: 0 };
  const fallbackOrder = [...context.characters.keys()].sort((a, b) => a - b).indexOf(index);
  return {
    name: translatedPlayerName(slot, context.partyData[slot] ?? null, playerOf(index, type), context.showDisplayNames),
    color: resolvePlayerColor(context.partyColors, slot, Math.max(0, fallbackOrder), context.partyData) ?? "#ffffff",
  };
};

/** Who applied a status: a party member's character, or an enemy. */
export const sourceName = (typeHash: number, context: CombatLogContext): string => {
  for (const [index, character] of context.characters) {
    if (typeof character === "string" && characterHash(character) === typeHash) {
      return combatLogMember(context, index, character).name;
    }
  }
  // A party member who dealt no damage, or a character outside the party.
  const character = Object.keys(CHARACTER_HASHES).find((name) => CHARACTER_HASHES[name] === typeHash >>> 0);
  if (character) {
    const member = context.partyData.find((data) => data?.characterType === character);
    return member ? combatLogMember(context, member.actorIndex, character).name : translateCharacterType(character);
  }
  return translateEnemyTypeId(typeHash);
};

/** Character hashes as the hook reports them (`CharacterType::from_hash` in the app). */
const CHARACTER_HASHES: { [character: string]: number } = {
  Pl0000: 0x26a4848a,
  Pl0100: 0x9498420d,
  Pl0200: 0x34d4fd8f,
  Pl0300: 0xf8d73d33,
  Pl0400: 0x7b5934ad,
  Pl0500: 0x443d46bb,
  Pl0600: 0xa9d6569e,
  Pl0700: 0xfba6615d,
  Pl0800: 0x63a7c3f0,
  Pl0900: 0xf96a90c2,
  Pl1000: 0x28ac1108,
  Pl1100: 0x94e2514e,
  Pl1200: 0x2b4aa114,
  Pl1300: 0xc97f3365,
  Pl1400: 0x601aa977,
  Pl1500: 0xbcc238de,
  Pl1600: 0xc3155079,
  Pl1700: 0xd16cfbde,
  Pl1800: 0x6fdd6932,
  Pl1900: 0x8056abcd,
  Pl2000: 0xf5755c0e,
  Pl2100: 0x9c89a455,
  Pl2200: 0x59db0cd9,
  Pl2300: 0xda5a8e25,
  Pl2400: 0x4c714f77,
  Pl2500: 0xe330418f,
  Pl2600: 0xe3d1be26,
  Pl2700: 0x91418145,
  Pl2800: 0x48adda36,
  Pl2900: 0x0a58fb4d,
};

const characterHash = (character: string): number | undefined => CHARACTER_HASHES[character];

const skillName = (entry: Extract<CombatLogEntry, { kind: "damage" }>): string => {
  const skill: SkillState = {
    actionType: entry.action,
    childCharacterType: entry.childType,
    hits: 1,
    minDamage: entry.damage,
    maxDamage: entry.damage,
    totalDamage: entry.damage,
    totalStunValue: 0,
    maxStunValue: 0,
    damageDetails: null,
  };
  return getSkillName(entry.sourceType, skill);
};

export const formatCombatLogEntry = (entry: CombatLogEntry, context: CombatLogContext): CombatLogLine => {
  const time = formatTime(entry.time);
  const kind = kindOf(entry);
  switch (entry.kind) {
    case "damage": {
      const { name, color } = combatLogMember(context, entry.sourceIndex, entry.sourceType);
      return {
        time,
        kind,
        color,
        actorIndex: entry.sourceIndex,
        text: t("ui.combat-log.damage", {
          source: name,
          skill: skillName(entry),
          target: translateEnemyTypeId(entry.targetType),
          damage: formatNumber(entry.damage),
        }),
      };
    }
    case "damageTaken": {
      const { name, color } = combatLogMember(context, entry.targetIndex, entry.targetType);
      return {
        time,
        kind,
        color,
        actorIndex: entry.targetIndex,
        text: t("ui.combat-log.damage-taken", {
          source: translateEnemyTypeId(entry.sourceType),
          target: name,
          damage: formatNumber(entry.damage),
        }),
      };
    }
    case "partBroken":
      return {
        time,
        kind,
        color: "#ffd43b",
        actorIndex: -1,
        text: t("ui.combat-log.part-broken", { enemy: translateEnemyTypeId(entry.enemyType) }),
      };
    case "statusApplied":
    case "statusRemoved": {
      const { name, color } = combatLogMember(context, entry.actorIndex);
      const status = statusName(entry.className);
      const text =
        entry.kind === "statusRemoved"
          ? t("ui.combat-log.status-removed", { actor: name, status })
          : entry.sourceType !== null && entry.sourceType !== undefined
            ? t("ui.combat-log.status-applied-by", {
                actor: name,
                status,
                source: sourceName(entry.sourceType, context),
              })
            : t("ui.combat-log.status-applied", { actor: name, status });
      return { time, kind, color, actorIndex: entry.actorIndex, text };
    }
    default: {
      const { name, color } = combatLogMember(context, entry.actorIndex);
      const key = {
        sbaReady: "sba-ready",
        sbaPerform: "sba-perform",
        sbaChain: "sba-chain",
        death: "death",
      }[entry.kind];
      return {
        time,
        kind,
        color,
        actorIndex: entry.actorIndex,
        text: t(`ui.combat-log.${key}`, { actor: name }),
      };
    }
  }
};

/** Plain text of the lines, one per row, for copying. */
export const combatLogText = (lines: CombatLogLine[]): string =>
  lines.map((line) => `[${line.time}] ${line.text}`).join("\n");

/** Every party member's character type by actor index, from damage dealt and taken. */
export const partyCharacters = (encounter: EncounterState): Map<number, CharacterType> => {
  const characters = new Map<number, CharacterType>();
  for (const player of Object.values(encounter.party)) characters.set(player.index, player.characterType);
  for (const taken of Object.values(encounter.damageTaken ?? {})) {
    if (!characters.has(taken.index)) characters.set(taken.index, taken.characterType);
  }
  // Members who only had statuses: known by the character hash their statuses carry.
  for (const [index, byClass] of Object.entries(encounter.statuses ?? {})) {
    const hash = Object.values(byClass).find((stats) => stats.actorType)?.actorType;
    if (hash === undefined || characters.has(Number(index))) continue;
    const character = Object.keys(CHARACTER_HASHES).find((name) => CHARACTER_HASHES[name] === hash >>> 0);
    characters.set(Number(index), character ?? { Unknown: hash });
  }
  return characters;
};
