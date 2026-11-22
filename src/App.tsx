import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { OverlayBridge } from "./overlay/OverlayBridge";

import Logs from "./pages/Logs";
import { IndexPage as LogIndexPage } from "./pages/logs/Index";
import { ViewPage as LogViewPage } from "./pages/logs/View";

import "./App.css";

export const App = () => {
  return (
    <BrowserRouter>
      {/* The live meter is drawn in game; this window only feeds it. */}
      <OverlayBridge />
      <Routes>
        <Route path="/" element={<Navigate to="/logs" replace />} />
        <Route path="/logs" element={<Logs />}>
          <Route index element={<LogIndexPage />} />
          <Route path=":id" element={<LogViewPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
};
