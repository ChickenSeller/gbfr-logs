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
