import { useEffect, useRef, useState } from "react";
import { meetingChips } from "../api";
import type { MeetingRow } from "../generated/types";
import { chipLookupKey, latestOnly, type HeldChips } from "./meetingCardModel";

const EMPTY: ReadonlyMap<number, HeldChips> = new Map();

/**
 * The Prep and Topic chips of recorded meetings, from one background lookup for all of them. It runs again only
 * when `chipLookupKey` changes (a list that re-reads every few seconds does not refetch), and a stale answer
 * never overwrites a newer one. Until it answers, the rows show without those chips.
 */
export function useMeetingChips(rows: Pick<MeetingRow, "id" | "hasSummary">[]): ReadonlyMap<number, HeldChips> {
  const key = chipLookupKey(rows);
  const [chips, setChips] = useState<ReadonlyMap<number, HeldChips>>(EMPTY);
  const latest = useRef(latestOnly());
  const ids = useRef<number[]>([]);
  ids.current = rows.map((r) => r.id);

  useEffect(() => {
    const wanted = ids.current;
    const ticket = latest.current.begin();
    if (wanted.length === 0) {
      setChips(EMPTY);
      return;
    }
    meetingChips(wanted).then(
      (found) => {
        if (!latest.current.isLatest(ticket)) return;
        // No answer (an older shell, the test bridge) reads as no chips, never as a crash.
        const list = Array.isArray(found) ? found : [];
        setChips(new Map(list.map((c) => [c.meetingId, { prepKey: c.prepKey ?? null, topic: c.topic ?? null }])));
      },
      () => {},
    );
  }, [key]);

  return chips;
}
