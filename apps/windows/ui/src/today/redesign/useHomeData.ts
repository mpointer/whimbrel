// What the redesigned Home reads that Today does not already load. Everything here runs only when Home is drawn in the
// redesign (the component is mounted only then), so a flag-off Today makes none of these calls.

import { useCallback, useEffect, useState } from "react";
import { approvalsCount, calendarAccounts, libraryMeeting, meetingList, todaySchedule, todayUpcoming, libraryCaptureApps, libraryDayBlocks, type CaptureApp, type DayBlock } from "../../api";
import type { MeetingRow, ScheduleView } from "../../generated/types";
import { meetingIntervals, peopleFrom, startOfDay } from "./homeModel";

const MINUTE = 60_000;

/** The day's memory blocks, cut around the meetings. Read again when the schedule changes and each minute. */
export function useDayBlocks(schedule: ScheduleView | null, now: number): { blocks: DayBlock[]; loaded: boolean } {
  const [blocks, setBlocks] = useState<DayBlock[]>([]);
  const [loaded, setLoaded] = useState(false);
  const minute = Math.floor(now / MINUTE);
  useEffect(() => {
    let live = true;
    const t0 = startOfDay(now);
    libraryDayBlocks(t0, now + MINUTE, meetingIntervals(schedule)).then(
      (b) => {
        if (!live) return;
        setBlocks(Array.isArray(b) ? b : []);
        setLoaded(true);
      },
      () => live && setLoaded(true),
    );
    return () => {
      live = false;
    };
    // `now` itself changes every 30 s; the minute is what moves the read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [schedule, minute]);
  return { blocks, loaded };
}

/** Whether a calendar is connected; null until known. */
export function useCalendarConnected(): boolean | null {
  const [connected, setConnected] = useState<boolean | null>(null);
  useEffect(() => {
    let live = true;
    calendarAccounts().then(
      (a) => live && setConnected((a?.emails?.length ?? 0) > 0),
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);
  return connected;
}

/** Routine actions waiting for approval. */
export function useApprovals(): number {
  const [n, setN] = useState(0);
  useEffect(() => {
    let live = true;
    approvalsCount().then(
      (c) => live && setN(c),
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);
  return n;
}

/** The apps seen in the last 30 days, loaded the first time the Apps filter opens. */
export function useCaptureApps(): { apps: CaptureApp[] | null; load: () => void } {
  const [apps, setApps] = useState<CaptureApp[] | null>(null);
  const load = useCallback(() => {
    const now = Date.now();
    libraryCaptureApps(now - 30 * 86_400_000, now + MINUTE, 12).then(
      (a) => setApps(Array.isArray(a) ? a : []),
      () => setApps([]),
    );
  }, []);
  return { apps, load };
}

/** The people the People filter offers, read when the chip is opened: attendees of today's and the coming calendar
 *  events, and of the recent meetings (the existing `meeting_list` and `library_meeting` commands, at most 20
 *  meetings). Nothing is stored. */
export function usePeople(): { people: string[]; load: () => void } {
  const [lists, setLists] = useState<string[][]>([]);
  const load = useCallback(() => {
    const none = <T,>(): T[] => [];
    Promise.all([todaySchedule().catch(() => null), todayUpcoming().catch(() => null), meetingList("").catch(() => none<MeetingRow>())]).then(async ([sched, up, rows]) => {
      const events = [...(sched?.events ?? []), ...(up?.days?.flatMap((d) => d.events) ?? [])];
      const held = await Promise.all(rows.slice(0, 20).map((m) => libraryMeeting(m.id).then((d) => d?.attendees ?? [], () => [] as string[])));
      setLists([...events.map((e) => e.attendees), ...held]);
    });
  }, []);
  return { people: peopleFrom(lists), load };
}
