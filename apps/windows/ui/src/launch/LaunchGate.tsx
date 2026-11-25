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
import { recheckRedesign, useRedesign } from "../redesign";
import { RedesignedLaunch } from "./redesign/RedesignedLaunch";
import { StartingScreen as StartingScreenR } from "./redesign/screens";
import { BlockingScreen, KeyUnavailableScreen, MigratingScreen, StartingScreen } from "./LaunchScreens";
import { NoStoreContext } from "./noStore";
import { RecoveryModel } from "./recoveryModel";
import { RecoveryView } from "./RecoveryView";
import { NOT_PRESSED, isCurrentView, nextProgress, pressReducer } from "./screenModel";
import { SetupModel } from "./setupModel";
import { SetupView } from "./SetupView";

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
  // Known before the first render (main.tsx asks the shell first), so neither look flashes before the other.
  const redesign = useRedesign();
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

  if (!stage || stage.kind === "starting") return redesign ? <StartingScreenR /> : <StartingScreen />;
  if (stage.kind === "opened" || stage.kind === "openedWithoutStore") return <Opened noStore={stage.kind === "openedWithoutStore"} />;
  if (stage.kind === "quitting") return redesign ? <StartingScreenR delayMs={0} /> : <StartingScreen delayMs={0} />;
  const busy = pressed || (view?.busy ?? false);
  const onContinue = () => {
    setPressed(true);
    setupContinue("launch").then(
      (handed) => (handed ? setup.clear() : setPressed(false)),
      () => setPressed(false),
    );
  };

  if (redesign) {
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

  return (
    <div className="launch">
      {stage.kind === "onboarding" ? (
        <SetupView
          model={setup}
          continueTitle={t("Continue")}
          continuing={pressed}
          onContinue={onContinue}
          onQuit={() => answer("quit")}
          quitSignal={quitSignal}
          notice={freshNotice}
        />
      ) : stage.kind === "migrating" ? (
        <MigratingScreen overall={overall} />
      ) : stage.kind === "recovery" ? (
        <RecoveryView reason={stage.reason} model={recovery} onQuit={() => answer("quit")} />
      ) : stage.kind === "keyUnavailable" ? (
        <KeyUnavailableScreen stillUnavailable={stage.stillUnavailable} busy={busy} onAnswer={answer} />
      ) : (
        <BlockingScreen screen={stage.screen} offers={stage.offers} lastAction={stage.lastAction} busy={busy} onAnswer={answer} />
      )}
    </div>
  );
}

/** The app, once the launch opened, drawn after the redesign switch was asked again, so neither shell flashes
 *  before the other. main.tsx asked before the first render, which on a fresh install is before the onboarding,
 *  while the shell cannot answer yet; without this the early false held until a restart. A second ask (a remount,
 *  React's strict-mode double effect) only reads the same setting again. The good-launch counter runs after the
 *  app's first render. */
function Opened({ noStore }: { noStore: boolean }) {
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    let live = true;
    void recheckRedesign().finally(() => {
      if (live) setAsked(true);
    });
    return () => {
      live = false;
    };
  }, []);
  return asked ? <OpenedApp noStore={noStore} /> : <StartingScreenFor />;
}

/** The starting screen in the look the switch has now. */
function StartingScreenFor() {
  return useRedesign() ? <StartingScreenR /> : <StartingScreen />;
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
