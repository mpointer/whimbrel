// The main window before the memory database is open (brief 2.7). It renders the launch's screens
// until the stage is `opened` (or `openedWithoutStore`), then the app. The stage is read from the
// shell on every mount and on every `launch://stage`, never kept from an event alone: the main
// window can be closed and rebuilt while the launch waits.

import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  launchFirstWindowRendered,
  launchFreshLibraryNotice,
  launchRecoveryChooseFile,
  launchRecoveryUnlockFile,
  launchRecoveryUnlockWords,
  launchRespond,
  launchState,
  setupBegin,
  setupContinue,
  setupHasFreshMaterial,
  setupSavedTo,
  setupSaveFile,
  setupStartOver,
  setupVerify,
} from "../api";
import { App } from "../App";
import type { LaunchAnswer, LaunchView } from "../generated/types";
import { t } from "../l10n";
import { RedesignedLaunch } from "./redesign/RedesignedLaunch";
import { StartingScreen } from "./redesign/screens";
import { NoStoreContext } from "./noStore";
import { RecoveryModel } from "./recoveryModel";
import { NOT_PRESSED, isCurrentView, nextProgress, pressReducer } from "./screenModel";
import { SetupModel } from "./setupModel";

// Asked once per window: the shell consumes the marker, so a remount (or React's strict-mode
// double effect) must reuse the first answer.
let freshLibraryNotice: Promise<string | null> | null = null;

/** After a Delete everything, the line the onboarding shows first (Mike, 2026-11-16); null otherwise. */
function useFreshLibraryNotice(onboarding: boolean): string | null {
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    if (!onboarding) return;
    let live = true;
    freshLibraryNotice ??= launchFreshLibraryNotice().catch(() => null);
    void freshLibraryNotice.then((n) => {
      if (live) setNotice(n);
    });
    return () => {
      live = false;
    };
  }, [onboarding]);
  return notice;
}

export function LaunchGate() {
  const [view, setView] = useState<LaunchView | null>(null);
  const [overall, setOverall] = useState(0);
  // Set when the user answered; cleared when the shell shows a screen again (its generation
  // moves, even for the same screen) or refuses the answer.
  const [press, dispatch] = useReducer(pressReducer, NOT_PRESSED);
  const pressed = press.pressed;
  const setPressed = (on: boolean) => dispatch({ kind: on ? "pressed" : "refused" });
  const [quitSignal, setQuitSignal] = useState(0);
  const shownGeneration = useRef<number | null>(null);

  useEffect(() => {
    let live = true;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const read = () =>
      launchState().then(
        (v) => {
          if (!live) return;
          // No answer yet (a shell still starting): ask again shortly.
          if (!v?.stage) {
            retry = setTimeout(read, 500);
            return;
          }
          // Two reads can resolve out of order: an older view never replaces a newer one.
          if (!isCurrentView(shownGeneration.current, v.generation)) return;
          shownGeneration.current = v.generation;
          dispatch({ kind: "view", generation: v.generation });
          const s = v.stage;
          if (s.kind === "migrating") setOverall((o) => nextProgress(o, s.overall));
          setView(v);
        },
        () => {
          // The shell is still starting: ask again shortly.
          if (live) retry = setTimeout(read, 500);
        },
      );
    void read();
    const stops = [
      listen("launch://stage", () => void read()),
      listen("launch://opened", () => void read()),
      listen<{ overall: number }>("launch://progress", (e) => setOverall((o) => nextProgress(o, e.payload.overall))),
      listen("launch://quit-requested", () => setQuitSignal((n) => n + 1)),
    ];
    return () => {
      live = false;
      clearTimeout(retry);
      stops.forEach((s) => void s.then((stop) => stop()));
    };
  }, []);

  const answer = (a: LaunchAnswer) => {
    setPressed(true);
    launchRespond(a).then(
      (accepted) => {
        if (!accepted) setPressed(false);
      },
      () => setPressed(false),
    );
  };

  const stage = view?.stage;
  const freshNotice = useFreshLibraryNotice(stage?.kind === "onboarding");
  const reusesKey = stage?.kind === "onboarding" ? stage.reusesKey : false;
  const setup = useMemo(
    () =>
      new SetupModel({
        begin: (p) => setupBegin("launch", p),
        saveFile: () => setupSaveFile("launch"),
        verify: (p) => setupVerify("launch", p),
        startOver: () => setupStartOver("launch"),
        savedTo: () => setupSavedTo("launch"),
        hasFreshMaterial: () => setupHasFreshMaterial("launch"),
        print: () => window.print(),
        reusesKey,
      }),
    [reusesKey],
  );
  const recovery = useMemo(
    () => new RecoveryModel({ chooseFile: launchRecoveryChooseFile, unlockFile: launchRecoveryUnlockFile, unlockWords: launchRecoveryUnlockWords }),
    [],
  );

  if (!stage || stage.kind === "starting") return <StartingScreen />;
  if (stage.kind === "opened" || stage.kind === "openedWithoutStore") return <OpenedApp noStore={stage.kind === "openedWithoutStore"} />;
  if (stage.kind === "quitting") return <StartingScreen delayMs={0} />;
  const busy = pressed || (view?.busy ?? false);
  const onContinue = () => {
    setPressed(true);
    setupContinue("launch").then(
      (handed) => (handed ? setup.clear() : setPressed(false)),
      () => setPressed(false),
    );
  };

  return (
    <RedesignedLaunch
      stage={stage}
      setup={setup}
      recovery={recovery}
      overall={overall}
      busy={busy}
      continuing={pressed}
      onAnswer={answer}
      onContinue={onContinue}
      continueTitle={t("Continue")}
      quitSignal={quitSignal}
      notice={freshNotice}
    />
  );
}

function OpenedApp({ noStore }: { noStore: boolean }) {
  useEffect(() => {
    if (!noStore) void launchFirstWindowRendered().catch(() => {});
  }, [noStore]);
  return (
    <NoStoreContext.Provider value={noStore}>
      <App />
    </NoStoreContext.Provider>
  );
}
