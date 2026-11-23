import { Box, Button, Checkbox, Group, Loader, Pagination, Select, Stack, Text } from "@mantine/core";
import { invoke } from "@tauri-apps/api";
import { useEffect, useMemo, useState } from "react";
import toast from "react-hot-toast";
import { useTranslation } from "react-i18next";

import {
  COMBAT_LOG_KINDS,
  CombatLogKind,
  combatLogMember,
  combatLogText,
  formatCombatLogEntry,
  kindOf,
  partyCharacters,
} from "@/combatLog";
import { CombatLogEntry, EncounterState, PlayerData } from "@/types";

const PAGE_SIZE = 500;

type Props = {
  id: string;
  encounter: EncounterState;
  partyData: Array<PlayerData | null>;
  playerColors: string[];
  showDisplayNames: boolean;
};

/** The saved encounter's full combat log, filterable by kind and party member. */
export const CombatLogTab = ({ id, encounter, partyData, playerColors, showDisplayNames }: Props) => {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<CombatLogEntry[] | null>(null);
  const [kinds, setKinds] = useState<CombatLogKind[]>([...COMBAT_LOG_KINDS]);
  const [actor, setActor] = useState<string | null>(null);
  const [page, setPage] = useState(1);

  useEffect(() => {
    setEntries(null);
    invoke<CombatLogEntry[]>("fetch_combat_log", { id: Number(id) })
      .then(setEntries)
      .catch((error) => {
        toast.error(String(error));
        setEntries([]);
      });
  }, [id]);

  const characters = useMemo(() => partyCharacters(encounter), [encounter]);
  const context = useMemo(
    () => ({ partyData, characters, partyColors: playerColors, showDisplayNames }),
    [partyData, characters, playerColors, showDisplayNames]
  );
  const lines = useMemo(() => {
    if (!entries) return [];
    return entries
      .filter((entry) => kinds.includes(kindOf(entry)))
      .map((entry) => formatCombatLogEntry(entry, context))
      .filter((line) => actor === null || String(line.actorIndex) === actor);
    // The language is a dependency: lines are translated when they are built.
  }, [entries, kinds, actor, context, i18n.language]);

  useEffect(() => setPage(1), [kinds, actor]);

  if (entries === null) {
    return (
      <Group mt="md">
        <Loader size="sm" />
        <Text size="sm">{t("ui.combat-log.loading")}</Text>
      </Group>
    );
  }

  const actorOptions = [...characters.keys()].map((index) => ({
    value: String(index),
    label: combatLogMember(context, index).name,
  }));
  const pages = Math.max(1, Math.ceil(lines.length / PAGE_SIZE));
  const shown = lines.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);

  const copy = () => {
    navigator.clipboard
      .writeText(combatLogText(lines))
      .then(() => toast.success(t("ui.copied-to-clipboard")))
      .catch((error) => toast.error(String(error)));
  };

  return (
    <Stack mt="md" gap="xs">
      <Group>
        <Checkbox.Group value={kinds} onChange={(value) => setKinds(value as CombatLogKind[])}>
          <Group gap="sm">
            {COMBAT_LOG_KINDS.map((kind) => (
              <Checkbox key={kind} value={kind} label={t(`ui.combat-log.kind-${kind}`)} />
            ))}
          </Group>
        </Checkbox.Group>
        <Select
          size="xs"
          clearable
          placeholder={t("ui.combat-log.all-players")}
          data={actorOptions}
          value={actor}
          onChange={setActor}
        />
        <Button size="xs" variant="default" onClick={copy} disabled={lines.length === 0}>
          {t("ui.combat-log.copy")}
        </Button>
        <Text size="xs" c="dimmed">
          {t("ui.combat-log.count", { count: lines.length })}
        </Text>
      </Group>
      {lines.length === 0 ? (
        <Text size="sm" c="dimmed">
          {t("ui.combat-log.empty")}
        </Text>
      ) : (
        <Box ff="monospace" fz="xs">
          {shown.map((line, i) => (
            <Text key={(page - 1) * PAGE_SIZE + i} size="xs" ff="monospace">
              <Text span c="dimmed" ff="monospace" size="xs">
                [{line.time}]{" "}
              </Text>
              <Text span c={line.color} ff="monospace" size="xs">
                {line.text}
              </Text>
            </Text>
          ))}
        </Box>
      )}
      {pages > 1 && <Pagination total={pages} value={page} onChange={setPage} size="sm" />}
    </Stack>
  );
};
