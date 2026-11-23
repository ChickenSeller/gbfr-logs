import { invoke } from "@tauri-apps/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

/**
 * Keeps what the app draws natively (tray menu, window titles) in the selected language. They
 * are created before any translation loads, so they start in English; this relabels them once
 * translations are in and again whenever the language changes.
 */
export const NativeLabels = () => {
  const { t, i18n } = useTranslation();

  useEffect(() => {
    const labels = {
      openLogs: t("ui.tray.open-logs"),
      // Keyed by the tray ids of protocol::OverlayWindow.
      overlayWindows: {
        toggle_overlay_meter: t("ui.tray.toggle-overlay-meter"),
        toggle_overlay_enemy_hp: t("ui.tray.toggle-overlay-enemy-hp"),
        toggle_overlay_damage_taken: t("ui.tray.toggle-overlay-damage-taken"),
        toggle_overlay_combat_log: t("ui.tray.toggle-overlay-combat-log"),
      },
      toggleOverlayMenu: t("ui.tray.toggle-overlay-menu"),
      resetWindows: t("ui.tray.reset-windows"),
      about: t("ui.tray.about"),
      quit: t("ui.tray.quit"),
      logsTitle: t("ui.windows.logs"),
      aboutTitle: t("ui.windows.about"),
    };
    invoke("set_native_labels", { labels }).catch(() => {});
  }, [t, i18n.language]);

  return null;
};
