import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./tokens.css";
import "./app.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { AlertView } from "./alert/AlertView";
import { PopupView } from "./popup/PopupView";
import { startSkin } from "./skin";

// Before the first render, so no window paints in the wrong skin. Every window runs it: the
// popup and the alert share this bundle, and hear about a change through storage.
startSkin();

// One bundle serves every window; each is told apart by its label.
const view = { popup: <PopupView />, alert: <AlertView /> }[getCurrentWindow().label] ?? <App />;

createRoot(document.getElementById("root")!).render(<StrictMode>{view}</StrictMode>);
