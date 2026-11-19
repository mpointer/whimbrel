// The capture state for the redesigned shell and Home. Reads and polls only while `active` (the redesign flag is on),
// so a flag-off window makes no extra calls.

import { useCallback, useEffect, useState } from "react";
import { capturePauseUntil, captureResumeNow, captureStatus } from "../../api";
import type { CaptureStatus } from "../../generated/types";
import { captureStateOf, pauseDeadline, type CaptureState, type PauseChoice } from "./captureModel";

const POLL_MS = 15_000;

export interface CaptureControl {
  /** Null until the shell has answered. */
  state: CaptureState | null;
  now: number;
  pause: (choice: PauseChoice) => void;
  resume: () => void;
}

export function useCapture(active: boolean): CaptureControl {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const load = useCallback(() => {
    setNow(Date.now());
    captureStatus().then(setStatus, () => {});
  }, []);

  useEffect(() => {
    if (!active) return;
    load();
    // The clock moves with the poll, so a pause that ends while the window is open is noticed within it.
    const timer = setInterval(load, POLL_MS);
    window.addEventListener("focus", load);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", load);
    };
  }, [active, load]);

  const pause = useCallback((choice: PauseChoice) => {
    capturePauseUntil(pauseDeadline(choice, Date.now())).then((s) => {
      setStatus(s);
      setNow(Date.now());
    }, () => {});
  }, []);
  const resume = useCallback(() => {
    captureResumeNow().then((s) => {
      setStatus(s);
      setNow(Date.now());
    }, () => {});
  }, []);

  return { state: captureStateOf(status, now), now, pause, resume };
}
