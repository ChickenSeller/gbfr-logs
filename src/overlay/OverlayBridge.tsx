import { usePrevious } from "@mantine/hooks";
import { invoke } from "@tauri-apps/api";
import { writeText } from "@tauri-apps/api/clipboard";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";

import { changeLanguagePreference } from "@/i18n";
import { SUPPORTED_LANGUAGES, SYSTEM_LANGUAGE, getLanguagePreference } from "@/language";
import { useMeterSettingsStore } from "@/stores/useMeterSettingsStore";
import {
  CombatLogEntry,
  EncounterState,
  EncounterUpdateEvent,
  OverlayCopyRequest,
  OverlayNotice,
  OverlaySettingsValues,
  PartyUpdateEvent,
  PlayerData,
} from "@/types";
import { buildFullEncounterText, buildSimpleEncounterText } from "@/utils";
import {
  OVERLAY_COMBAT_LOG_LINES,
  buildOverlaySettings,
  buildOverlayState,
  meterSettingsFromOverlay,
} from "./buildOverlayState";

const DEFAULT_ENCOUNTER_STATE: EncounterState = {
  totalDamage: 0,
  dps: 0,
  startTime: 0,
  endTime: 1,
  party: {},
  targets: {},
  status: "Waiting",
};

const EMPTY_PARTY: Array<PlayerData | null> = [null, null, null, null];

const LANGUAGES = Object.entries(SUPPORTED_LANGUAGES).map(([value, label]) => ({ value, label }));

/**
 * Adds a live entry, keeping the last `OVERLAY_COMBAT_LOG_LINES * 2`. Times restart with each
 * encounter, so an entry from well before the last one starts a new log.
 */
export const appendCombatLogEntry = (log: CombatLogEntry[], entry: CombatLogEntry): CombatLogEntry[] => {
  const last = log[log.length - 1];
  if (last && entry.time + 1000 < last.time) return [entry];
  return [...log.slice(-(OVERLAY_COMBAT_LOG_LINES * 2 - 1)), entry];
};

const notifyOverlay = (text: string, level: OverlayNotice["level"]) => {
  const notice: OverlayNotice = { text, level };
  invoke("push_overlay_notice", { notice }).catch(() => {});
};

/**
 * Feeds the in-game meter, which replaced the desktop meter window.
 *
 * Mounted once in the logs window, which stays alive while hidden: it tracks the live
 * encounter, turns it into overlay snapshots with the settings page's options, answers the
 * overlay's copy requests, applies settings edited in game to the settings store and forwards
 * status messages into the game. The app paces delivery,
 * so this works while the window is hidden and its timers are throttled.
 */
export const OverlayBridge = () => {
  const { t, i18n } = useTranslation();
  // The saved choice ("system" or a language code). Kept in state because switching between
  // "system" and the language it resolves to does not change i18n.language.
  const [languagePreference, setLanguagePreference] = useState(getLanguagePreference);
  const languagePreferenceRef = useRef(languagePreference);
  languagePreferenceRef.current = languagePreference;
  const [encounterState, setEncounterState] = useState<EncounterState>(DEFAULT_ENCOUNTER_STATE);
  const [partyData, setPartyData] = useState<Array<PlayerData | null>>(EMPTY_PARTY);
  const [lastPartyData, setLastPartyData] = useState<Array<PlayerData | null>>(EMPTY_PARTY);
  const previousStatus = usePrevious(encounterState.status);
  // Recent combat log entries; the overlay only shows the last lines.
  const [combatLog, setCombatLog] = useState<CombatLogEntry[]>([]);
  // Enemies the overlay tracks before they take damage, or whose damage lands on parts.
  const [trackedEnemies, setTrackedEnemies] = useState<number[]>([]);

  const settings = useMeterSettingsStore(
    useShallow((state) => ({
      color_1: state.color_1,
      color_2: state.color_2,
      color_3: state.color_3,
      color_4: state.color_4,
      transparency: state.transparency,
      show_display_names: state.show_display_names,
      streamer_mode: state.streamer_mode,
      show_full_values: state.show_full_values,
      use_condensed_skills: state.use_condensed_skills,
      open_log_on_save: state.open_log_on_save,
      overlay_columns: state.overlay_columns,
    }))
  );

  // A finished encounter keeps showing the party it was fought with, as the meter did.
  useEffect(() => {
    if (previousStatus === "InProgress" && encounterState.status === "Stopped") {
      setLastPartyData(partyData);
    }
  }, [previousStatus, encounterState.status, partyData]);
  const visiblePartyData = encounterState.status === "Stopped" ? lastPartyData : partyData;

  // Copy requests and notices arrive outside React's render flow.
  const current = useRef({ encounterState, partyData: visiblePartyData });
  current.current = { encounterState, partyData: visiblePartyData };

  useEffect(() => {
    const languages = [{ value: SYSTEM_LANGUAGE, label: t("ui.language-system") }, ...LANGUAGES];
    const overlaySettings = buildOverlaySettings(settings, languagePreference, languages, (key) => t(key));
    const state = buildOverlayState(encounterState, visiblePartyData, overlaySettings, combatLog, trackedEnemies);
    // Without an injected hook the app keeps the latest snapshot for later.
    invoke("push_overlay_state", { state }).catch(() => {});
  }, [encounterState, visiblePartyData, settings, t, i18n.language, languagePreference, combatLog, trackedEnemies]);

  useEffect(() => {
    const listeners = [
      listen("encounter-update", (event: EncounterUpdateEvent) => setEncounterState(event.payload)),
      listen("overlay-tracked-enemies", (event: { payload: number[] }) =>
        setTrackedEnemies((previous) =>
          previous.length === event.payload.length && previous.every((hash, i) => hash === event.payload[i])
            ? previous
            : event.payload
        )
      ),
      listen("encounter-party-update", (event: PartyUpdateEvent) => setPartyData(event.payload)),
      // Cleared from the in-game menu: the next hit starts a new encounter.
      listen("encounter-cleared", () => {
        setEncounterState(DEFAULT_ENCOUNTER_STATE);
        setCombatLog([]);
        notifyOverlay(t("ui.overlay-cleared"), "Success");
      }),
      listen("combat-log-entry", (event: { payload: CombatLogEntry }) =>
        setCombatLog((log) => appendCombatLogEntry(log, event.payload))
      ),
      listen("on-area-enter", (event: EncounterUpdateEvent) => {
        setEncounterState(event.payload.status === "Waiting" ? DEFAULT_ENCOUNTER_STATE : event.payload);
        setCombatLog([]);
        notifyOverlay(t("ui.on-area-enter"), "Success");
      }),
      // Same effect as editing the settings page; the next snapshot echoes the new values.
      listen("overlay-settings-change", (event: { payload: OverlaySettingsValues }) => {
        const values = event.payload;
        useMeterSettingsStore.getState().set(meterSettingsFromOverlay(values));
        const preference = values.language;
        const valid = preference === SYSTEM_LANGUAGE || preference in SUPPORTED_LANGUAGES;
        if (valid && preference !== languagePreferenceRef.current) {
          languagePreferenceRef.current = preference;
          setLanguagePreference(preference);
          changeLanguagePreference(preference);
        }
      }),
      listen("encounter-saved", () => notifyOverlay(t("ui.successful-save"), "Success")),
      listen("diagnostics-exported", (event: { payload: string }) => {
        toast.success(t("ui.diagnostics.exported", { path: event.payload }));
        notifyOverlay(t("ui.diagnostics.exported", { path: event.payload }), "Success");
      }),
      listen("diagnostics-export-failed", (event: { payload: string }) => {
        toast.error(t("ui.diagnostics.failed", { error: event.payload }));
        notifyOverlay(t("ui.diagnostics.failed", { error: event.payload }), "Error");
      }),
      listen("encounter-saved-error", (event: { payload: string }) =>
        notifyOverlay(t("ui.unsuccessful-save", { error: event.payload }), "Error")
      ),
      listen("success-alert", (event: { payload: string }) => notifyOverlay(event.payload, "Success")),
      listen("error-alert", (event: { payload: string }) => notifyOverlay(event.payload, "Error")),
      // The game has focus when these arrive, so the native clipboard API is used: the web
      // clipboard API rejects writes from an unfocused page.
      listen("overlay-copy-request", async (event: { payload: OverlayCopyRequest }) => {
        const { kind, sort } = event.payload;
        const { encounterState, partyData } = current.current;
        const build = kind === "Full" ? buildFullEncounterText : buildSimpleEncounterText;
        const text = build(sort.column, sort.descending ? "desc" : "asc", encounterState, partyData);

        if (text === null) {
          notifyOverlay("Nothing to copy!", "Error");
          return;
        }
        try {
          await writeText(text);
          notifyOverlay(t("ui.copied-to-clipboard"), "Success");
        } catch (error) {
          notifyOverlay(`Copy failed: ${error}`, "Error");
        }
      }),
    ];

    return () => {
      listeners.forEach((listener) => listener.then((unlisten) => unlisten()));
    };
  }, [t, i18n]);

  return null;
};
