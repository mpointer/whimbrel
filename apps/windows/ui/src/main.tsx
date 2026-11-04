import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./tokens.css";
import "./app.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { languageSettings } from "./api";
import { L10nProvider, setLanguage, useLanguage } from "./l10n";
import { AlertView } from "./alert/AlertView";
import { QuestionAlertView } from "./alert/QuestionAlertView";
import { PopupView } from "./popup/PopupView";
import { startSkin } from "./skin";

// Before the first render, so no window paints in the wrong skin. Every window runs it: the
// popup and the alert share this bundle, and hear about a change through storage.
startSkin();

const label = getCurrentWindow().label;

// One bundle serves every window; each is told apart by its label. The view is a fresh element on
// every render, so a language change re-renders the whole window without remounting it.
function Root() {
  const lang = useLanguage();
  const view =
    label === "popup" ? <PopupView /> : label === "alert" ? <AlertView /> : label === "question-alert" ? <QuestionAlertView /> : <App />;
  return <L10nProvider lang={lang}>{view}</L10nProvider>;
}

// The language first, so nothing paints in the wrong one. The popup and the alert are created fresh
// each time they open, so they pick up a change made in Settings without being told.
languageSettings()
  .then((s) => s && setLanguage(s.uiLanguage))
  .catch(() => {})
  .finally(() =>
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <Root />
      </StrictMode>,
    ),
  );
