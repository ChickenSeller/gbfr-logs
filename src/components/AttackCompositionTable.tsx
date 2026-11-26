import { Box, Progress, Table, Text, Tooltip } from "@mantine/core";
import { useTranslation } from "react-i18next";

import { ATTACK_CATEGORIES, ATTACK_CATEGORY_COLORS, AttackCategory, attackComposition } from "@/attackCategories";
import { EncounterState, PlayerData } from "@/types";
import {
  formatInPartyOrder,
  hasOwnSkillName,
  humanizeNumbers,
  resolvePlayerColor,
  translatedPlayerName,
} from "@/utils";

type Props = {
  encounter: EncounterState;
  partyData: Array<PlayerData | null>;
  playerColors: string[];
  showDisplayNames: boolean;
};

/** Each player's damage split by attack type: a stacked bar plus the share of every type. */
export const AttackCompositionTable = ({ encounter, partyData, playerColors, showDisplayNames }: Props) => {
  const { t } = useTranslation();
  // Most damage first, like the overview; colours still follow each member's party slot.
  const players = formatInPartyOrder(encounter.party).sort((a, b) => b.totalDamage - a.totalDamage);

  return (
    <Table striped>
      <Table.Thead>
        <Table.Tr>
          <Table.Th>{t("ui.logs.name")}</Table.Th>
          {ATTACK_CATEGORIES.map((category) => (
            <Table.Th key={category}>
              <Box display="flex" style={{ alignItems: "center", gap: 6 }}>
                <Box w={10} h={10} style={{ background: ATTACK_CATEGORY_COLORS[category], borderRadius: 2 }} />
                {t(`ui.attack-types.${category}`)}
              </Box>
            </Table.Th>
          ))}
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {players.map((player) => {
          const partySlotIndex = partyData.findIndex((member) => member?.actorIndex === player.index);
          const composition = attackComposition(player, hasOwnSkillName);
          const total = ATTACK_CATEGORIES.reduce((sum, category) => sum + composition[category], 0);
          const share = (category: AttackCategory) => (total > 0 ? (composition[category] / total) * 100 : 0);

          return (
            <Table.Tr key={player.index}>
              <Table.Td>
                <Text size="sm" c={resolvePlayerColor(playerColors, partySlotIndex, player.partyIndex, partyData)}>
                  {translatedPlayerName(partySlotIndex, partyData[partySlotIndex] ?? null, player, showDisplayNames)}
                </Text>
                <Progress.Root size="md" mt={4}>
                  {ATTACK_CATEGORIES.filter((category) => composition[category] > 0).map((category) => (
                    <Tooltip
                      key={category}
                      label={`${t(`ui.attack-types.${category}`)} ${share(category).toFixed(1)}%`}
                    >
                      <Progress.Section value={share(category)} color={ATTACK_CATEGORY_COLORS[category]} />
                    </Tooltip>
                  ))}
                </Progress.Root>
              </Table.Td>
              {ATTACK_CATEGORIES.map((category) => {
                const [value, unit] = humanizeNumbers(composition[category]);
                return (
                  <Table.Td key={category}>
                    <Text size="sm">{share(category).toFixed(1)}%</Text>
                    <Text size="xs" c="dimmed">
                      {value}
                      {unit}
                    </Text>
                  </Table.Td>
                );
              })}
            </Table.Tr>
          );
        })}
      </Table.Tbody>
    </Table>
  );
};
