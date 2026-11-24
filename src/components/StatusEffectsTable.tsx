import { Badge, NumberFormatter, Stack, Table, Text } from "@mantine/core";
import { useTranslation } from "react-i18next";

import { CombatLogContext, combatLogMember, partyCharacters, sourceName } from "@/combatLog";
import { formatSeconds, isDebuff, isHighlighted, sortStatuses, statusName } from "@/statuses";
import { EncounterState, PlayerData, StatusStats } from "@/types";

type Props = {
  encounter: EncounterState;
  partyData: Array<PlayerData | null>;
  playerColors: string[];
  showDisplayNames: boolean;
};

const SourceList = ({ stats, context }: { stats: StatusStats; context: CombatLogContext }) => (
  <>
    {Object.entries(stats.sources)
      .sort(([, a], [, b]) => b - a)
      .map(([hash, count]) => (
        <Text size="xs" key={hash}>
          {sourceName(Number(hash), context)} ×{count}
        </Text>
      ))}
  </>
);

/** Per party member: every status effect they had, debuffs first, with counts and durations. */
export const StatusEffectsTable = ({ encounter, partyData, playerColors, showDisplayNames }: Props) => {
  const { t } = useTranslation();
  const statuses = encounter.statuses ?? {};
  const context: CombatLogContext = {
    partyData,
    characters: partyCharacters(encounter),
    partyColors: playerColors,
    showDisplayNames,
  };
  const members = Object.keys(statuses)
    .map(Number)
    .sort((a, b) => a - b);

  if (members.length === 0) {
    return (
      <Text size="sm" c="dimmed">
        {t("ui.status-effects.empty")}
      </Text>
    );
  }

  return (
    <Stack>
      {members.map((index) => {
        const { name, color } = combatLogMember(context, index);
        const rows = sortStatuses(Object.values(statuses[index]));
        return (
          <Stack key={index} gap={4}>
            <Text fw={700} c={color}>
              {name}
            </Text>
            <Table striped>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>{t("ui.status-effects.status")}</Table.Th>
                  <Table.Th>{t("ui.status-effects.count")}</Table.Th>
                  <Table.Th>{t("ui.status-effects.total")}</Table.Th>
                  <Table.Th>{t("ui.status-effects.average")}</Table.Th>
                  <Table.Th>{t("ui.status-effects.longest")}</Table.Th>
                  <Table.Th title={t("ui.status-effects.damage-while-active-hint")}>
                    {t("ui.status-effects.damage-while-active")}
                  </Table.Th>
                  <Table.Th>{t("ui.status-effects.sources")}</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {rows.map((stats) => (
                  <Table.Tr key={stats.className}>
                    <Table.Td>
                      <Badge size="xs" mr={6} color={isDebuff(stats.className) ? "red" : "teal"} variant="light">
                        {isDebuff(stats.className) ? t("ui.status-effects.debuffs") : t("ui.status-effects.buffs")}
                      </Badge>
                      <Text
                        span
                        size="sm"
                        title={stats.className}
                        c={isHighlighted(stats) ? "yellow" : undefined}
                        fw={isHighlighted(stats) ? 700 : undefined}
                      >
                        {statusName(stats.className)}
                      </Text>
                    </Table.Td>
                    <Table.Td>{stats.count}</Table.Td>
                    <Table.Td>{formatSeconds(stats.totalMs)}</Table.Td>
                    <Table.Td>{formatSeconds(stats.count > 0 ? stats.totalMs / stats.count : 0)}</Table.Td>
                    <Table.Td>{formatSeconds(stats.maxMs)}</Table.Td>
                    <Table.Td>
                      <NumberFormatter thousandSeparator value={stats.damageWhileActive ?? 0} />
                    </Table.Td>
                    <Table.Td>
                      <SourceList stats={stats} context={context} />
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </Stack>
        );
      })}
    </Stack>
  );
};
