import { describe, expect, it } from "vitest";

import { attackCategory, attackComposition, isNormalAttack } from "./attackCategories";
import { SkillState } from "./types";

const skill = (
  actionType: SkillState["actionType"],
  totalDamage: number,
  childCharacterType = "Pl0000"
): SkillState => ({
  actionType,
  childCharacterType,
  hits: 1,
  minDamage: totalDamage,
  maxDamage: totalDamage,
  totalDamage,
  totalStunValue: 0,
  maxStunValue: 0,
  damageDetails: null,
});

describe("isNormalAttack", () => {
  it("splits at 1000 for most characters", () => {
    expect(isNormalAttack("Pl0000", 100)).toBe(true);
    expect(isNormalAttack("Pl0000", 999)).toBe(true);
    expect(isNormalAttack("Pl0000", 1001)).toBe(false);
  });

  it("knows Narmaya's stance attacks", () => {
    expect(isNormalAttack("Pl1400", 1100)).toBe(true);
    expect(isNormalAttack("Pl1400", 4000)).toBe(true);
  });

  it("knows Sandalphon's and Maglielle's finishers", () => {
    expect(isNormalAttack("Pl2100", 6000)).toBe(true);
    expect(isNormalAttack("Pl2100", 2400)).toBe(false);
    expect(isNormalAttack("Pl2500", 9999)).toBe(true);
  });

  it("uses shared normal ids only without a skill of the character's own", () => {
    expect(isNormalAttack("Pl0300", 5000, false)).toBe(true);
    expect(isNormalAttack("Pl2600", 5000, true)).toBe(false);
  });
});

describe("attackCategory", () => {
  it("maps the game's own action types", () => {
    expect(attackCategory("SBA", "Pl0000")).toBe("sba");
    expect(attackCategory("LinkAttack", "Pl0000")).toBe("link");
    expect(attackCategory({ SupplementaryDamage: 1 }, "Pl0000")).toBe("supplementary");
    expect(attackCategory({ DamageOverTime: 1 }, "Pl0000")).toBe("dot");
    expect(attackCategory({ Group: "normal-attack" }, "Pl0000")).toBeNull();
  });
});

describe("attackComposition", () => {
  it("sums damage per category", () => {
    const composition = attackComposition({
      characterType: "Pl0000",
      skillBreakdown: [
        skill({ Normal: 100 }, 10),
        skill({ Normal: 110 }, 5),
        skill({ Normal: 1001 }, 30),
        skill("SBA", 50),
        skill("LinkAttack", 7),
        skill({ SupplementaryDamage: 3 }, 2),
      ],
    });
    expect(composition).toEqual({ normal: 15, skill: 30, sba: 50, link: 7, supplementary: 2, dot: 0 });
  });
});
