import { useCallback, useEffect, useState } from "react";
import { meetingRecordStartEvent, nextMeetingPrep, todaySchedule } from "../api";
import { describeError, isCommandError } from "../errors";
import type { NextMeetingView, ScheduleView } from "../generated/types";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export interface ScheduleState {
  schedule: ScheduleView | null;
  next: NextMeetingView | null;
  now: number;
  starting: string | null;
  error: string | null;
  start: (eventId: string) => void;
}

/**
 * Today's schedule and the next meeting's prep, read when Today is shown and on window focus, with a 30 s
 * tick that moves the countdowns (not the data). `onChanged` runs after a recording is started here, so Today
 * can re-read what it shows.
 */
export function useSchedule(onChanged: () => void): ScheduleState {
  const [schedule, setSchedule] = useState<ScheduleView | null>(null);
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [starting, setStarting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setNow(Date.now());
    todaySchedule()
      .then((s) => {
        setSchedule(s);
        setError(null);
      })
      .catch((e: unknown) => setError(message(e)));
    nextMeetingPrep()
      .then(setNext)
      .catch(() => setNext(null));
  }, []);

  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    const tick = setInterval(() => setNow(Date.now()), 30_000);
    return () => {
      window.removeEventListener("focus", load);
      clearInterval(tick);
    };
  }, [load]);

  const start = useCallback(
    (eventId: string) => {
      setStarting(eventId);
      setError(null);
      meetingRecordStartEvent(eventId)
        .then(() => {
          load();
          onChanged();
        })
        .catch((e: unknown) => setError(message(e)))
        .finally(() => setStarting(null));
    },
    [load, onChanged],
  );

  return { schedule, next, now, starting, error, start };
}
