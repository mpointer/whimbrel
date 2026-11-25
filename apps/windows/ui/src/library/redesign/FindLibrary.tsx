// The Library tab with the redesign flag on: Find (the default), Recent, Episodes and Digest. Find stays mounted while another view or a
// detail shows, so its results and its conversation are where they were when you come back. A search or a question from the header, from
// any tab, opens Find with the words, also when Library is already showing.

import { useEffect, useRef, useState } from "react";
import { t } from "../../l10n";
import { WSegmented } from "../../system";
import { Detail, type Selection } from "../Detail";
import { EpisodesView, DigestPanel, RecentView } from "./LibraryLists";
import { FindPage } from "./FindPage";
import type { FindOpen } from "./findModel";
import { useHeader } from "./findStore";

type View = "find" | "recent" | "episodes" | "digest";

export function FindLibrary({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const [view, setView] = useState<View>("find");
  const [selected, setSelected] = useState<(Selection & { from: string | null }) | null>(null);
  // Bumped when a detail is closed: Recent and Episodes are read again, since the detail can change what they list (hide an app, delete).
  const [round, setRound] = useState(0);
  const detail = useRef<HTMLDivElement>(null);
  const header = useHeader();
  const opener = useRef<string | null>(null);
  const bar = useRef<HTMLDivElement>(null);

  // A search or a question from the header: Find, with the words.
  const seq = header.request?.seq ?? 0;
  useEffect(() => {
    if (seq === 0) return;
    setView("find");
    setSelected(null);
  }, [seq]);

  const open = (target: FindOpen) => {
    opener.current = document.activeElement?.getAttribute("data-open-key") ?? null;
    setSelected({ kind: target.kind, id: target.id, line: target.line, from: opener.current });
    // The button that was pressed is hidden now: focus moves to the detail, which is read from its start.
    requestAnimationFrame(() => detail.current?.focus());
  };
  const back = () => {
    const key = opener.current;
    setSelected(null);
    setRound((n) => n + 1);
    // Focus goes back to the button that opened it, or to the view switch when that card is gone.
    requestAnimationFrame(() => {
      const button = key ? document.querySelector<HTMLElement>(`[data-open-key="${key}"]`) : null;
      (button ?? bar.current?.querySelector<HTMLElement>("[role=radio][aria-checked=true]") ?? null)?.focus();
    });
  };
  const choose = (next: View) => {
    setSelected(null);
    setView(next);
  };

  return (
    <div className="library th-find">
      {selected ? (
        <div ref={detail} tabIndex={-1} className="th-find-detail">
          <Detail selection={selected} onBack={back} />
        </div>
      ) : null}
      <div hidden={!!selected}>
        <div className="th-find-views" ref={bar}>
          <WSegmented
            label={t("Library view")}
            value={view}
            options={[
              { value: "find", label: t("Find") },
              { value: "recent", label: t("Recent") },
              { value: "episodes", label: t("Episodes") },
              { value: "digest", label: t("Digest") },
            ]}
            onChange={choose}
          />
        </div>
        <FindPage hidden={view !== "find" || !!selected} onOpen={open} onOpenSettings={onOpenSettings} />
        {view === "recent" ? <RecentView key={`recent-${round}`} onOpen={open} /> : null}
        {view === "episodes" ? <EpisodesView key={`episodes-${round}`} onOpen={open} /> : null}
        {view === "digest" ? <DigestPanel /> : null}
      </div>
    </div>
  );
}
