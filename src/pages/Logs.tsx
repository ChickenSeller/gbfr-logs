import { useMeterSettingsStore } from "@/stores/useMeterSettingsStore";
import "./Logs.css";

import { AppShell } from "@mantine/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { Toaster } from "react-hot-toast";
import { Outlet, useNavigate } from "react-router-dom";

const Layout = () => {
  const { open_log_on_save } = useMeterSettingsStore((state) => ({ open_log_on_save: state.open_log_on_save }));

  const navigate = useNavigate();

  useEffect(() => {
    const saveListener = listen("encounter-saved", (event: { payload: number | null }) => {
      if (event.payload && open_log_on_save) {
        navigate(`/logs/${event.payload}`);
      }
    });

    return () => {
      saveListener.then((f) => f());
    };
  }, [navigate, open_log_on_save]);

  return (
    <div className="log-window">
      {/* The log list is the only page; a log view links back to it. Credits are in the in-game menu. */}
      <AppShell padding="sm">
        <AppShell.Main>
          <Outlet />
        </AppShell.Main>
      </AppShell>
      <Toaster
        position="bottom-center"
        toastOptions={{
          style: {
            borderRadius: "10px",
            backgroundColor: "#252525",
            color: "#fff",
            fontSize: "14px",
          },
        }}
      />
    </div>
  );
};

export default Layout;
