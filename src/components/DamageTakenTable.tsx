import { NumberFormatter, Progress, Table, Text } from "@mantine/core";
import { useTranslation } from "react-i18next";

import { ComputedPlayerState, DamageTakenState, EncounterState, PlayerData } from "@/types";
import { resolvePlayerColor, translateEnemyTypeId, translatedPlayerName } from "@/utils";

type Props = {
  encounter: EncounterState;
  partyData: Array<PlayerData | null>;
  playerColors: string[];
  showDisplayNames: boolean;
};

/** The player row `translatedPlayerName` expects, for members who only took damage. */
const asPlayer = (taken: DamageTakenState, partyIndex: number): ComputedPlayerState => ({
  index: taken.index,
  characterType: taken.characterType,
  totalDamage: 0,
  dps: 0,
  sba: 0,
  totalStunValue: 0,
  stunPerSecond: 0,
  lastDamageTime: 0,
  skillBreakdown: [],
  partyIndex,
  percentage: 0,
});

/** Damage each party member took, most first, with the enemies that dealt it. */
export const DamageTakenTable = ({ encounter, partyData, playerColors, showDisplayNames }: Props) => {
  const { t } = useTranslation();
  const rows = Object.values(encounter.damageTaken ?? {}).sort((a, b) => b.totalDamage - a.totalDamage);
  const top = rows[0]?.totalDamage ?? 0;

  if (rows.length === 0) {
    return (
      <Text size="sm" c="dimmed">
        {t("ui.damage-taken.empty")}
      </Text>
    );
  }

  return (
    <Table striped>
      <Table.Thead>
        <Table.Tr>
          <Table.Th>{t("ui.logs.name")}</Table.Th>
          <Table.Th>{t("ui.damage-taken.total")}</Table.Th>
          <Table.Th>{t("ui.damage-taken.hits")}</Table.Th>
          <Table.Th>{t("ui.damage-taken.max-hit")}</Table.Th>
          <Table.Th>{t("ui.damage-taken.sources")}</Table.Th>
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {rows.map((taken, i) => {
          const partySlotIndex = partyData.findIndex((member) => member?.actorIndex === taken.index);
          const color = resolvePlayerColor(playerColors, partySlotIndex, i);
          const sources = Object.values(taken.sources).sort((a, b) => b.totalDamage - a.totalDamage);

          return (
            <Table.Tr key={taken.index}>
              <Table.Td>
                <Text size="sm" c={color}>
                  {translatedPlayerName(
                    partySlotIndex,
                    partyData[partySlotIndex] ?? null,
                    asPlayer(taken, i),
                    showDisplayNames
                  )}
                </Text>
                <Progress value={top > 0 ? (taken.totalDamage / top) * 100 : 0} color={color} size="sm" mt={4} />
              </Table.Td>
              <Table.Td>
                <NumberFormatter thousandSeparator value={taken.totalDamage} />
              </Table.Td>
              <Table.Td>{taken.hits}</Table.Td>
              <Table.Td>
                <NumberFormatter thousandSeparator value={taken.maxHit} />
              </Table.Td>
              <Table.Td>
                {sources.map((source) => (
                  <Text size="xs" key={source.rawSourceType}>
                    {translateEnemyTypeId(source.rawSourceType)}:{" "}
                    <NumberFormatter thousandSeparator value={source.totalDamage} /> ({source.hits})
                  </Text>
                ))}
              </Table.Td>
            </Table.Tr>
          );
        })}
      </Table.Tbody>
    </Table>
  );
};
