import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
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

/** How often the schedule is re-read on its own, so a meeting that ends or is recorded elsewhere shows up. */
const RELOAD_MS = 60_000;

/**
 * Today's schedule and the next meeting's prep, read when Today is shown, on window focus, after each
 * calendar round (`calendar-synced`) and every minute, with a 30 s tick that moves the countdowns and the
 * Now marker. `onChanged` runs after a recording is started here, so Today can re-read what it shows.
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
    const reload = setInterval(load, RELOAD_MS);
    const unlisten = listen("calendar-synced", load);
    return () => {
      window.removeEventListener("focus", load);
      clearInterval(tick);
      clearInterval(reload);
      void unlisten.then((stop) => stop());
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
