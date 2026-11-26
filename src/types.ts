/**
 * CharacterType represents the type of character that a player can be.
 *
 * Examples:
 * - `"Pl1000"`
 * - `"Pl1800"`
 * - `{ Unknown: 0xF546E414 }`
 */
export type CharacterType = string | { Unknown: number };

/**
 * EnemyType represents the type of enemy.
 *
 * Examples:
 * - `"Em1000"`
 * - `"Em1200"`
 * - `{ Unknown: 0xF546E414 }`
 */
export type EnemyType = string | { Unknown: number };

/**
 * ActionType represents the type of action that a skill can be.
 *
 * Examples:
 * - `"LinkAttack"` - Link Attack
 * - `"SBA"` - Skybound Art
 * - `{ SupplementaryAttack: 113 }` (as its key, object with a number representing the skill number)
 * - `{ Normal: 113 }` (as its key, object with a number representing the skill number)
 */
export type ActionType =
  | "LinkAttack"
  | "SBA"
  | { SupplementaryDamage: number }
  | { DamageOverTime: number }
  | { Normal: number }
  | { Group: string };

export type DamageModifierKind = "Attack" | "Defense" | "DamageLimit" | "BonusAttack" | "Amplify";

export type DamageStatusContribution = {
  statusName: string;
  kind: DamageModifierKind;
  category: number;
  averageValue: number;
  activeHits: number;
  multiplierContribution: number;
};

export type AverageDamageDetails = {
  hits: number;
  totalDamage: number;
  supplementaryDamage: number;
  normalizationDamage: number;
  effectiveMultiplier: number;
  recognizedMultiplier: number;
  damageLimitContribution: number;
  amplifyContribution: number;
  attackDefenseContribution: number;
  supplementaryContribution: number;
  unattributedContribution: number;
  averageDamageLimitMultiplier: number;
  averageAmplifyMultiplier: number;
  averageAttackDefenseMultiplier: number;
  averageCriticalRate: number;
  averageBaseDamage: number;
  averageDamageCap: number;
  cappedHits: number;
  statuses: DamageStatusContribution[];
};

export type SkillState = {
  /** ActionType of the skill */
  actionType: ActionType;
  /** For some characters, the skill can be a child of another character type. */
  childCharacterType: CharacterType;
  /** Number of total hits of the skill */
  hits: number;
  /** Minimum damage of the skill */
  minDamage: number | null;
  /** Maximum damage of the skill */
  maxDamage: number | null;
  /** Total damage of the skill */
  totalDamage: number;
  /** Total stun value of the skill hits */
  totalStunValue: number;
  /** Maximum recorded stun value of the skill */
  maxStunValue: number;
  /** Damage modifiers averaged across captured hits for this skill */
  damageDetails: AverageDamageDetails | null;
};

export type ComputedSkillState = SkillState & {
  /** Damage contribution as a percentage of the total */
  percentage: number;
};

export type ComputedSkillGroup = {
  /** ActionType of the skill */
  actionType: ActionType;
  /** For some characters, the skill can be a child of another character type. */
  childCharacterType: CharacterType;
  /** Number of total hits of the skill */
  hits: number;
  /** Minimum damage of the skill */
  minDamage: number | null;
  /** Maximum damage of the skill */
  maxDamage: number | null;
  /** Total damage of the skill */
  totalDamage: number;
  /** Damage contribution as a percentage of the total */
  percentage: number;
  /** Skills */
  skills?: ComputedSkillState[];
  /** Total stun value of the skill hits */
  totalStunValue: number;
  /** Maximum recorded stun value of the skill */
  maxStunValue: number;
  /** Groups do not expose a combined damage-detail average. */
  damageDetails: null;
};

export type PlayerState = {
  /** Unique ID for this player */
  index: number;
  /** Character type of this player. (Pl1000 / Pl1800 / ..) */
  characterType: CharacterType;
  /** Total damage dealt */
  totalDamage: number;
  /** DPS over the encounter time */
  dps: number;
  /** Amount of SBA Gauge (0.0 - 1000.0) */
  sba: number;
  /** Total stun value */
  totalStunValue: number;
  /** Stun per second over the encounter time */
  stunPerSecond: number;
  /** Time of the last damage dealt */
  lastDamageTime: number;
  /** Stats for individual skills logged */
  skillBreakdown: SkillState[];
};

export type ComputedPlayerState = PlayerState & {
  /** Damage contribution as a percentage of the total */
  percentage: number;
  /** Actual party index */
  partyIndex: number;
};

export type EnemyState = {
  /** Enemy index */
  index: number;
  /** Enemy type */
  targetType: EnemyType;
  /** Total damage done to this target */
  totalDamage: number;
  /** Hash ID of the enemy type */
  rawTargetType: number;
};

export type EncounterStatus = "Waiting" | "InProgress" | "Stopped";

export type EncounterState = {
  /** Total damage dealt in the whole encounter */
  totalDamage: number;
  /** Total DPS dealt over the encounter time */
  dps: number;
  /** The time of the encounter's first damage instance (UTC milliseconds since epoch) */
  startTime: number;
  /** The time of the encounter's last known damage instance (UTC milliseconds since epoch) */
  endTime: number;
  /** Represents the players in the encounter */
  party: Record<string, PlayerState>;
  /** Status of the encounter */
  status: EncounterStatus;
  /** Targets for this encounter */
  targets: Record<number, EnemyState>;
  /** Damage enemies dealt to each party member, keyed by actor index. Missing in old logs. */
  damageTaken?: Record<number, DamageTakenState>;
  /** Status effects per party member (actor index) and class name. Missing in old logs. */
  statuses?: Record<number, Record<string, StatusStats>>;
  /** Link attacks and SBA chains each member (actor index) joined and missed. Missing in old
   * logs. */
  missed?: Record<number, MissStats>;
};

/** `parser::v1::missed::MissStats`. */
export type MissStats = {
  links: number;
  missedLinks: number;
  sbas: number;
  missedChains: number;
  deaths: number;
  downedMs: number;
  autoRevives: number;
};

/** One entry of the combat log (`parser::v1::combat_log::CombatLogEntry`). `time` is in
 * milliseconds since the start of the encounter. */
export type CombatLogEntry =
  | {
      kind: "damage";
      time: number;
      sourceIndex: number;
      sourceType: CharacterType;
      childType: CharacterType;
      targetType: number;
      action: ActionType;
      damage: number;
    }
  | {
      kind: "damageTaken";
      time: number;
      sourceType: number;
      targetIndex: number;
      targetType: CharacterType;
      damage: number;
    }
  | { kind: "sbaReady" | "sbaPerform" | "sbaChain" | "death"; time: number; actorIndex: number }
  | { kind: "statusApplied"; time: number; actorIndex: number; className: string; sourceType: number | null }
  | { kind: "statusRemoved"; time: number; actorIndex: number; className: string }
  | { kind: "partBroken"; time: number; enemyType: number; partId: number };

/** Statistics of one status class on one party member. */
export type StatusStats = {
  /** RTTI class name, e.g. `StatusAilmentPoison` */
  className: string;
  typeId: number;
  count: number;
  /** Total time active, in milliseconds */
  totalMs: number;
  /** Longest single application, in milliseconds */
  maxMs: number;
  /** Applications per source type hash */
  sources: Record<number, number>;
  /** Applied by an enemy's grab at least once */
  grab?: boolean;
  /** Character hash of the member who had it */
  actorType?: number;
  /** Damage taken while it was active; a hit counts for every status active at the time */
  damageWhileActive: number;
};

export type DamageTakenState = {
  /** Actor index, as `PlayerState.index` */
  index: number;
  characterType: CharacterType;
  totalDamage: number;
  hits: number;
  maxHit: number;
  /** Per enemy type hash */
  sources: Record<number, DamageTakenSource>;
};

export type DamageTakenSource = {
  rawSourceType: number;
  totalDamage: number;
  hits: number;
  maxHit: number;
};

/** Display-ready meter snapshot sent to the in-game overlay (`protocol::OverlayState`). */
export type OverlayState = {
  status: EncounterStatus;
  startTime: number;
  endTime: number;
  totalDamage: number;
  dps: number;
  settings: OverlaySettings;
  /** Party order; the overlay sorts them itself. */
  players: OverlayPlayer[];
  /** Translated names of the enemies hit so far; the overlay reads their HP in game. */
  enemies: OverlayEnemy[];
  /** Attack types of `OverlayPlayer.attackDamage`, in that order. */
  attackTypes: OverlayAttackType[];
  /** Damage taken per party member, most first, filtered by streamer mode. */
  damageTaken: OverlayDamageTaken[];
  damageTakenLabels: {
    title: string;
    hits: string;
    maxHit: string;
    missedLinks: string;
    missedChains: string;
    deaths: string;
    autoRevives: string;
    downedTime: string;
  };
  /** Latest combat log lines, oldest first. */
  combatLog: OverlayLogLine[];
};

export type OverlayLogLine = {
  time: string;
  text: string;
  /** 0xRRGGBB */
  color: number;
};

export type OverlayDamageTaken = {
  name: string;
  color: number;
  totalDamage: number;
  hits: number;
  maxHit: number;
  /** Debuffs and ailments received, longest total first */
  debuffs: { name: string; count: number; totalMs: number; damage: number; highlight: boolean }[];
  missedLinks: number;
  missedChains: number;
  deaths: number;
  downedMs: number;
  autoRevives: number;
};

export type OverlayAttackType = {
  label: string;
  /** Segment colour as 0xRRGGBB. */
  color: number;
};

export type OverlayEnemy = {
  typeHash: number;
  name: string;
};

/** The settings page's meter options, with what the in-game settings menu needs to edit them. */
export type OverlaySettings = {
  values: OverlaySettingsValues;
  languages: Array<{ value: string; label: string }>;
  /** Every value column with its translated header and description. */
  columns: Array<{ column: MeterColumns; label: string; description: string }>;
  labels: OverlaySettingsLabels;
};

/** Editable values; the overlay sends them back when they change in game. */
export type OverlaySettingsValues = {
  language: string;
  /** Party slot 1-4 colours as 0xRRGGBB. */
  partyColors: number[];
  transparency: number;
  showDisplayNames: boolean;
  streamerMode: boolean;
  showFullValues: boolean;
  useCondensedSkills: boolean;
  openLogOnSave: boolean;
  columns: MeterColumns[];
};

export type OverlaySettingsLabels = {
  title: string;
  language: string;
  partyColors: string[];
  transparency: string;
  showDisplayNames: string;
  streamerMode: string;
  streamerModeDescription: string;
  showFullValues: string;
  showFullValuesDescription: string;
  useCondensedSkills: string;
  useCondensedSkillsDescription: string;
  openLogOnSave: string;
  openLogOnSaveDescription: string;
  columns: string;
};

export type OverlayPlayer = {
  /** Actor index; identifies the row while it is expanded. */
  index: number;
  partyOrder: number;
  /** Name exactly as the meter shows it. */
  name: string;
  /** Bar colour as 0xRRGGBB. */
  color: number;
  totalDamage: number;
  dps: number;
  percentage: number;
  sba: number;
  totalStunValue: number;
  stunPerSecond: number;
  skills: OverlaySkill[];
  /** Damage per attack type, in the order of `OverlayState.attackTypes`. */
  attackDamage: number[];
};

export type OverlaySkill = {
  name: string;
  hits: number;
  totalDamage: number;
  minDamage: number | null;
  maxDamage: number | null;
  percentage: number;
  /** Skills inside a condensed group. */
  children: OverlaySkill[];
};

/** Mirrors `protocol::OverlayNotice`. */
export type OverlayNotice = {
  text: string;
  level: "Success" | "Error";
};

/** Mirrors `protocol::OverlayCopyRequest`: which export, in the overlay's sort order. */
export type OverlayCopyRequest = {
  kind: "Simple" | "Full";
  sort: { column: MeterColumns; descending: boolean };
};

export type EncounterUpdateEvent = {
  event: string;
  payload: EncounterState;
};

export type EncounterResetEvent = {
  event: string;
  payload: EncounterState;
};

export type Sigil = {
  firstTraitId: number;
  firstTraitLevel: number;
  secondTraitId: number;
  secondTraitLevel: number;
  sigilId: number;
  equippedCharacter: number;
  sigilLevel: number;
  acquisitionCount: number;
  notificationEnum: number;
};

export type WeaponInfo = {
  weaponId: number;
  starLevel: number;
  plusMarks: number;
  awakeningLevel: number;
  trait1Id: number;
  trait1Level: number;
  trait2Id: number;
  trait2Level: number;
  trait3Id: number;
  trait3Level: number;
  wrightstoneId: number;
  weaponLevel: number;
  weaponHp: number;
  weaponAttack: number;
};

export type Overmastery = {
  id: number;
  flags: number;
  value: number;
};

export type OvermasteryInfo = {
  overmasteries: Overmastery[];
};

export type PlayerStats = {
  level: number;
  totalHp: number;
  totalAttack: number;
  stunPower: number;
  criticalRate: number;
  totalPower: number;
};

export type PlayerData = {
  actorIndex: number;
  displayName: string;
  characterName: string;
  characterType: CharacterType;
  sigils: Sigil[];
  isOnline: boolean;
  weaponInfo: WeaponInfo | null;
  overmasteryInfo: OvermasteryInfo | null;
  playerStats: PlayerStats | null;
};

export type PartyUpdateEvent = {
  event: string;
  payload: Array<PlayerData | null>;
};

export enum MeterColumns {
  Name = "name",
  DPS = "dps",
  TotalDamage = "damage",
  DamagePercentage = "damage-percentage",
  SBA = "sba",
  TotalStunValue = "total-stun-value",
  StunPerSecond = "stun-per-second",
}

export type SortType = MeterColumns;

export type LogSortType = "time" | "duration" | "quest-elapsed-time";
export type SortDirection = "asc" | "desc";

export type Log = {
  id: number;
  name: string;
  time: number;
  duration: number;
  version: number;
  primaryTarget: EnemyType | null;
  p1Name: string | null;
  p1Type: string | null;
  p2Name: string | null;
  p2Type: string | null;
  p3Name: string | null;
  p3Type: string | null;
  p4Name: string | null;
  p4Type: string | null;
  questId: number | null;
  questElapsedTime: number | null;
  questCompleted: boolean;
};

export type SBAEvent = [
  number,
  (
    | { OnAttemptSBA: { actor_index: number } }
    | { OnPerformSBA: { actor_index: number } }
    | { OnContinueSBAChain: { actor_index: number } }
  ),
];

export type DeathEvent = [number, { OnDeathEvent: { actor_index: number; death_counter: number } }];
