/**
 * Status effects (buffs, debuffs, ailments) as the game names them: RTTI class names such as
 * `StatusAilmentPoison`. Translated names live under `ui.statuses`; anything else is shown
 * as a readable form of the class name.
 */
import { t } from "i18next";

import { StatusStats } from "./types";

/** Statuses that hurt the one who has them: ailments and debuffs. */
export const isDebuff = (className: string): boolean =>
  /Ailment|Debuff/.test(className) && !/ClearAilment|ProtectAilment/.test(className);

/** `StatusAilmentPoison` -> `Ailment Poison`. */
export const readableClassName = (className: string): string =>
  className
    .replace(/^Status/, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .trim();

export const statusName = (className: string): string =>
  t([`ui.statuses.${className}`], { defaultValue: readableClassName(className) });

/** Debuffs first, then by total time active. */
export const sortStatuses = (stats: StatusStats[]): StatusStats[] =>
  [...stats].sort((a, b) => Number(isDebuff(b.className)) - Number(isDebuff(a.className)) || b.totalMs - a.totalMs);

export const formatSeconds = (ms: number): string => `${(ms / 1000).toFixed(1)}s`;
