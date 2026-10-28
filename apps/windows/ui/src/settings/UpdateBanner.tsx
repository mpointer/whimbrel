import { useEffect, useState } from "react";
import { installUpdate, updateReady } from "../api";
import { describeError, isCommandError } from "../errors";

// The shell downloads at launch and then daily, so a slow poll is enough.
const POLL_MS = 10 * 60 * 1000;

/** "Whimbrel X is ready": shown once the shell has downloaded an update, like the Mac's banner. */
export function UpdateBanner() {
  const [version, setVersion] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const poll = () => updateReady().then(setVersion).catch(() => {});
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, []);

  if (!version) return null;
  return (
    <section className="card notice" role="region" aria-label="Update ready">
      <p>
        Whimbrel {version} is ready. Restarting installs it and reopens Whimbrel; a meeting being recorded is stopped and
        saved first.
      </p>
      {error && <p role="alert">{error}</p>}
      <div className="toolbar">
        <button
          className="mode"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setError(null);
            installUpdate().catch((e: unknown) => {
              setBusy(false);
              setError(isCommandError(e) ? describeError(e) : String(e));
            });
          }}
        >
          {busy ? "Restarting…" : "Restart now"}
        </button>
      </div>
    </section>
  );
}
