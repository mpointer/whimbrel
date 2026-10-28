import { useRef, useState } from "react";
import { todayRecap } from "../api";
import { describeError, isCommandError } from "../errors";
import type { TodayRecap } from "../generated/types";
import { recapNote } from "./todayModel";

/** A recap's text, shared by Today and the Library's Digest mode. */
export function RecapBody({ recap }: { recap: TodayRecap }) {
  return (
    <>
      <p className="recap">{recap.summary}</p>
      {recap.openLoops.length > 0 && (
        <>
          <h3 className="section-label">Still open from today</h3>
          <ul className="recap-loops">
            {recap.openLoops.map((loop, i) => (
              <li key={i}>{loop}</li>
            ))}
          </ul>
        </>
      )}
      <p className="muted small">{recapNote(recap)}</p>
    </>
  );
}

/**
 * The Library's Digest mode (the Mac's "Generate Today's Digest"): today's recap on
 * request, from the same per-day cache Today uses, with Regenerate to write it again.
 */
export function DigestView() {
  const [recap, setRecap] = useState<TodayRecap | null>(null);
  const [writing, setWriting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);

  const generate = (refresh: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setWriting(true);
    setError(null);
    todayRecap(refresh)
      .then(setRecap)
      .catch((e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e)))
      .finally(() => {
        inFlight.current = false;
        setWriting(false);
      });
  };

  return (
    <div className="today stack-sm">
      <div className="field-row">
        <button className="mode mode--active" onClick={() => generate(false)} disabled={writing}>
          {writing ? "Writing…" : "Generate today's digest"}
        </button>
        {recap && !recap.emptyDay && (
          <button className="mode" onClick={() => generate(true)} disabled={writing}>
            Regenerate
          </button>
        )}
      </div>
      {error && (
        <div role="alert" className="card">
          {error}
        </div>
      )}
      {recap && (
        <section className="card" aria-label="Today's digest">
          <RecapBody recap={recap} />
        </section>
      )}
    </div>
  );
}
