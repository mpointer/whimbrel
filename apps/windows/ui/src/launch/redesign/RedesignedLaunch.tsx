// The launch's stages in the Thread look, behind the hidden redesign flag. LaunchGate decides (it reads the flag
// before the first render) and keeps everything else: what it reads from the shell, the models, the answers and the
// calls. This only draws a stage with the new components.

import type { LastActionView, LaunchAnswer, LaunchStageView } from "../../generated/types";
import type { RecoveryModel } from "../recoveryModel";
import type { SetupModel } from "../setupModel";
import { RecoveryScreen } from "./RecoveryScreen";
import { SetupScreen } from "./SetupScreen";
import { BlockingScreen, KeyUnavailableScreen, MigratingScreen } from "./screens";
import "./redesign.css";

/** The stages the launch draws itself: the rest (starting, quitting, opened) are decided by LaunchGate. */
export type DrawnStage = Extract<LaunchStageView, { kind: "onboarding" | "migrating" | "recovery" | "keyUnavailable" | "screen" }>;

export interface RedesignedLaunchProps {
  stage: DrawnStage;
  setup: SetupModel;
  recovery: RecoveryModel;
  /** The migration's progress, 0 to 1. */
  overall: number;
  /** An answer is in flight, or the shell is working. */
  busy: boolean;
  /** The setup's Continue is in flight. */
  continuing: boolean;
  onAnswer: (a: LaunchAnswer) => void;
  onContinue: () => void;
  continueTitle: string;
  quitSignal: number;
  notice: string | null;
}

export function RedesignedLaunch(p: RedesignedLaunchProps) {
  const { stage } = p;
  switch (stage.kind) {
    case "onboarding":
      return (
        <SetupScreen
          model={p.setup}
          continueTitle={p.continueTitle}
          continuing={p.continuing}
          onContinue={p.onContinue}
          onQuit={() => p.onAnswer("quit")}
          quitSignal={p.quitSignal}
          notice={p.notice}
        />
      );
    case "migrating":
      return <MigratingScreen overall={p.overall} />;
    case "recovery":
      return <RecoveryScreen reason={stage.reason} model={p.recovery} onQuit={() => p.onAnswer("quit")} />;
    case "keyUnavailable":
      return <KeyUnavailableScreen stillUnavailable={stage.stillUnavailable} busy={p.busy} onAnswer={p.onAnswer} />;
    case "screen":
      return <BlockingScreen screen={stage.screen} offers={stage.offers} lastAction={stage.lastAction as LastActionView | null | undefined} busy={p.busy} onAnswer={p.onAnswer} />;
  }
}
