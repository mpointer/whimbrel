// Recent, Episodes and Digest in the Thread look: the same cards and day grouping as the Find page, with the toggles and actions they have today.

import { useCallback, useEffect, useMemo, useState } from "react";
import { libraryEpisodes, libraryRecent, librarySettings, setEpisodeAppHidden, setShowScreenCaptures, todayRecap } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { LibraryRow, TodayRecap } from "../../generated/types";
import { t, useL10n } from "../../l10n";
import { WButton, WCheckbox, WNotice } from "../../system";
import { RecapBody } from "../../today/DigestView";
import { FindCardView } from "./FindCard";
import { DayLine } from "./FindResultsView";
import { cardOfRow, type FindOpen } from "./findModel";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Rows as cards by day. */
function Rows({ rows, label, onOpen }: { rows: LibraryRow[]; label: string; onOpen: (open: FindOpen) => void }) {
  const { locale } = useL10n();
  const now = useMemo(() => new Date(), []);
  const cards = useMemo(() => rows.map(cardOfRow), [rows]);
  return (
    <DayLine
      label={label}
      cards={cards}
      order="newest"
      render={(c) => (
        <li key={`${c.kind}-${c.id}`} className="th-find-item">
          <FindCardView card={c} now={now} locale={locale} onOpen={onOpen} />
        </li>
      )}
    />
  );
}

export function RecentView({ onOpen }: { onOpen: (open: FindOpen) => void }) {
  const [rows, setRows] = useState<LibraryRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCaptures, setShowCaptures] = useState(true);
  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    libraryRecent().then(
      (r) => {
        setRows(r);
        setLoading(false);
      },
      (e: unknown) => {
        setError(message(e));
        setLoading(false);
      },
    );
  }, []);
  useEffect(() => {
    load();
    librarySettings().then((s) => setShowCaptures(s.showScreenCaptures), () => {});
  }, [load]);
  const toggle = (show: boolean) =>
    setShowScreenCaptures(show).then(
      (s) => {
        setShowCaptures(s.showScreenCaptures);
        load();
      },
      (e: unknown) => setError(message(e)),
    );
  return (
    <div className="th-find-main">
      <div className="th-find-toggles">
        <WCheckbox label={t("Show screen captures")} checked={showCaptures} onChange={(v) => void toggle(v)} />
        <WButton onClick={load}>{t("Refresh")}</WButton>
      </div>
      {error ? <WNotice tone="error">{error}</WNotice> : null}
      {loading ? (
        <p className="th-find-status" role="status">
          {t("Loading…")}
        </p>
      ) : rows.length === 0 && !error ? (
        <section className="th-find-empty">
          <p>{t("Nothing captured yet. Captures appear here as you work, from this PC and, once sync is on, your other devices.")}</p>
        </section>
      ) : (
        <Rows rows={rows} label={t("Recent")} onOpen={onOpen} />
      )}
    </div>
  );
}

export function EpisodesView({ onOpen }: { onOpen: (open: FindOpen) => void }) {
  const [rows, setRows] = useState<LibraryRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [includeHidden, setIncludeHidden] = useState(false);
  const [hiddenApps, setHiddenApps] = useState<string[]>([]);
  const load = useCallback((hidden: boolean) => {
    setLoading(true);
    setError(null);
    libraryEpisodes(hidden).then(
      (r) => {
        setRows(r);
        setLoading(false);
      },
      (e: unknown) => {
        setError(message(e));
        setLoading(false);
      },
    );
  }, []);
  useEffect(() => load(includeHidden), [load, includeHidden]);
  useEffect(() => {
    librarySettings().then((s) => setHiddenApps(s.hiddenEpisodeApps), () => {});
  }, []);
  const unhide = (app: string) =>
    setEpisodeAppHidden(app, false).then(
      (s) => {
        setHiddenApps(s.hiddenEpisodeApps);
        load(includeHidden);
      },
      (e: unknown) => setError(message(e)),
    );
  return (
    <div className="th-find-main">
      <div className="th-find-toggles">
        <WCheckbox label={t("Show hidden episodes")} checked={includeHidden} onChange={setIncludeHidden} />
        {hiddenApps.length > 0 ? (
          <span className="th-find-toggles" role="group" aria-label={t("Hidden apps")}>
            <span className="th-find-small">{t("Hidden apps:")}</span>
            {hiddenApps.map((app) => (
              <WButton key={app} title={t("Show this app's episodes again")} onClick={() => void unhide(app)}>
                {app} ×
              </WButton>
            ))}
          </span>
        ) : null}
      </div>
      {error ? <WNotice tone="error">{error}</WNotice> : null}
      {loading ? (
        <p className="th-find-status" role="status">
          {t("Loading…")}
        </p>
      ) : rows.length === 0 && !error ? (
        <section className="th-find-empty">
          <p>{t("No episodes yet. They form from a few minutes of captures in one app.")}</p>
        </section>
      ) : (
        <Rows rows={rows} label={t("Episodes")} onOpen={onOpen} />
      )}
    </div>
  );
}

/** Today's recap on request, from the same per-day cache Today uses, with Regenerate to write it again. */
export function DigestPanel() {
  const [recap, setRecap] = useState<TodayRecap | null>(null);
  const [writing, setWriting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generate = (refresh: boolean) => {
    if (writing) return;
    setWriting(true);
    setError(null);
    todayRecap(refresh)
      .then(setRecap)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setWriting(false));
  };
  return (
    <div className="th-find-main">
      <div className="th-find-toggles">
        <WButton variant="primary" onClick={() => generate(false)} disabled={writing}>
          {writing ? t("Writing…") : t("Generate today's digest")}
        </WButton>
        {recap && !recap.emptyDay ? (
          <WButton onClick={() => generate(true)} disabled={writing}>
            {t("Regenerate")}
          </WButton>
        ) : null}
      </div>
      {error ? <WNotice tone="error">{error}</WNotice> : null}
      {recap ? (
        <section className="th-find-answer" aria-label={t("Today's digest")}>
          <RecapBody recap={recap} />
        </section>
      ) : null}
    </div>
  );
}
