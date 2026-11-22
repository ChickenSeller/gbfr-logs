import { appWindow } from "@tauri-apps/api/window";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { NativeLabels } from "./components/NativeLabels";
import { OverlayBridge } from "./overlay/OverlayBridge";

import { AboutPage } from "./pages/About";
import Logs from "./pages/Logs";
import { IndexPage as LogIndexPage } from "./pages/logs/Index";
import { ViewPage as LogViewPage } from "./pages/logs/View";

import "./App.css";

export const App = () => {
  return (
    <BrowserRouter>
      {/* The live meter is drawn in game and fed by the logs window, which lives as long as the
          app. Other windows (About) must not feed it a second time. */}
      {appWindow.label === "logs" && (
        <>
          <OverlayBridge />
          <NativeLabels />
        </>
      )}
      <Routes>
        <Route path="/" element={<Navigate to="/logs" replace />} />
        <Route path="/about" element={<AboutPage />} />
        <Route path="/logs" element={<Logs />}>
          <Route index element={<LogIndexPage />} />
          <Route path=":id" element={<LogViewPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
};
