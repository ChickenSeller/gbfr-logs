/**
 * Damage by attack type: normal attacks, skills, Skybound Arts, link attacks, supplementary
 * damage and damage over time.
 *
 * The game tells SBA, link attacks, supplementary damage and DoT apart itself (`ActionType`).
 * Normal attacks and skills share `ActionType.Normal(id)`; the action id tells them apart. For
 * most characters ids below 1000 are normal attacks and the rest are skills. The exceptions
 * below were checked against the skill names in `lang/en/ui.json`.
 */
import { ActionType, CharacterType, PlayerState, SkillState } from "./types";

export const ATTACK_CATEGORIES = ["normal", "skill", "sba", "link", "supplementary", "dot"] as const;
export type AttackCategory = (typeof ATTACK_CATEGORIES)[number];

/** Colours of the stacked bar segments, also used by the in-game meter. */
export const ATTACK_CATEGORY_COLORS: { [category in AttackCategory]: string } = {
  normal: "#8D99AE",
  skill: "#4C9AFF",
  sba: "#FFAB00",
  link: "#36B37E",
  supplementary: "#B37FEB",
  dot: "#FF5630",
};

/** First action id of the skills for most characters. */
const FIRST_SKILL_ID = 1000;

/** Normal attacks with ids in the skill range, per character. */
const NORMAL_ATTACK_EXCEPTIONS: { [character: string]: (id: number) => boolean } = {
  // Narmaya: both stances' combos, finishers and stance shifts use 1000-4000.
  Pl1400: (id) => id < 5000,
  // Sandalphon: Combo Finisher (Swords), also with Chromatic Wings.
  Pl2100: (id) => id === 6000 || id === 6001,
  // Maglielle: Combo Finisher Bonus.
  Pl2500: (id) => id === 9999,
};

/**
 * Shared ids that are normal actions (Ether Round, Charged Shot, dodge). They only apply when the
 * character has no skill of its own with that id: Beatrix's 5000 is a skill.
 */
const SHARED_NORMAL_IDS = new Set([5000, 5010, 4294967295]);

const characterKey = (character: CharacterType): string | null => (typeof character === "string" ? character : null);

/** Whether `id` is a normal attack of `character`. `hasOwnSkill` tells if the character lists it. */
export const isNormalAttack = (character: CharacterType, id: number, hasOwnSkill = true): boolean => {
  const key = characterKey(character);
  const exception = key ? NORMAL_ATTACK_EXCEPTIONS[key] : undefined;
  if (exception?.(id)) return true;
  if (!hasOwnSkill && SHARED_NORMAL_IDS.has(id)) return true;
  return id < FIRST_SKILL_ID;
};

export const attackCategory = (
  actionType: ActionType,
  character: CharacterType,
  hasOwnSkill: (character: CharacterType, id: number) => boolean = () => true
): AttackCategory | null => {
  if (actionType === "LinkAttack") return "link";
  if (actionType === "SBA") return "sba";
  if (typeof actionType !== "object") return null;
  if ("SupplementaryDamage" in actionType) return "supplementary";
  if ("DamageOverTime" in actionType) return "dot";
  if ("Normal" in actionType) {
    const id = actionType.Normal;
    return isNormalAttack(character, id, hasOwnSkill(character, id)) ? "normal" : "skill";
  }
  // Groups only exist after condensing; composition is computed from the raw skills.
  return null;
};

export type AttackComposition = { [category in AttackCategory]: number };

export const emptyComposition = (): AttackComposition => ({
  normal: 0,
  skill: 0,
  sba: 0,
  link: 0,
  supplementary: 0,
  dot: 0,
});

/**
 * Total damage of a player per attack type, from the raw (not condensed) skill breakdown.
 * Child actors (Ferry's pets, Id's dragon form) are classified with their own skill ids.
 */
export const attackComposition = (
  player: Pick<PlayerState, "characterType" | "skillBreakdown">,
  hasOwnSkill?: (character: CharacterType, id: number) => boolean
): AttackComposition => {
  const composition = emptyComposition();
  for (const skill of player.skillBreakdown as SkillState[]) {
    const character = skill.childCharacterType ?? player.characterType;
    const category = attackCategory(skill.actionType, character, hasOwnSkill);
    if (category) composition[category] += skill.totalDamage;
  }
  return composition;
};
