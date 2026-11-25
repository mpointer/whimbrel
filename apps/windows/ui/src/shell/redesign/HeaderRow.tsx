// The header row at the top of the content on every section: the section's title on the left (on Home with the date
// under it), the search field in the middle, the capture state on the right. On Home and on the Find page the filter chips sit under it.
// Enter searches (on this PC, sending nothing to a model) and shows results on the Find page. The Ask control beside Search, with its
// Ctrl+Enter keycap, asks; it is named for where the answer would come from, by the same route preview the Find page's Ask button reads,
// and Enter never asks. On the Find page the page's own Ask button is the one Ask, so the header keeps Search alone. With no AI provider
// set up the control says so, as the page's does. Where the field is narrow the control shrinks to its icon (its name and tooltip stay)
// before the field is squeezed. The words and the filters are held in the Find store, so the page reads what the field holds.

import { useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { openAiSettings } from "../../api";
import { t } from "../../l10n";
import type { RoutePreviewView } from "../../generated/types";
import { useAskState } from "../../library/redesign/askState";
import { useContinuedConversation } from "../../library/redesign/askStore";
import { askLabel, askReadiness, filterInput, filterKey, filterWords } from "../../library/redesign/findModel";
import { headerState, requestFind, setHeaderFilter, setHeaderText, useHeader } from "../../library/redesign/findStore";
import { useRoutePreview } from "../../library/redesign/routePreview";
import { WAskBar, WButton, WIconButton } from "../../system";
import { HomeFilters } from "../../today/redesign/HomeFilters";
import { hasFilter, NO_FILTER } from "../../today/redesign/homeModel";
import { useCaptureApps, usePeople } from "../../today/redesign/useHomeData";
import type { ElementChoices } from "../../today/redesign/elements";
import { escapeCloses, searchShown } from "./searchCollapse";

export interface HeaderRowProps {
  title: string;
  /** The date under the title, on Home. */
  date?: string;
  /** The capture state control, drawn on every section once the shell has answered. */
  capture: ReactNode;
  ask: ElementChoices["ask"];
  placeholder: string;
  /** Home: draw the filter chips under the row (the Find page draws them as well). */
  filters: boolean;
  now: number;
  inputRef: RefObject<HTMLInputElement>;
  /** Opens the Library tab on Find; the request itself is in the Find store. */
  onFind: () => void;
  /** Counts up each time Ctrl+K is pressed: the field opens (where it is collapsed to an icon) and takes focus. */
  focusTick?: number;
  /** Screen 6: where an Ask from this section goes. True when the section answered it (on Today, in a conversation); false goes to the Find page. */
  onAsk?: (query: string, shown: RoutePreviewView | null, filtered: boolean) => boolean;
}

export function HeaderRow(p: HeaderRowProps) {
  const header = useHeader();
  const { text, filter } = header;
  // At a middle width the field is an icon button until it is asked for (shell.css); elsewhere it is always drawn and this changes nothing.
  const [asked, setAsked] = useState(false);
  const [wantFocus, setWantFocus] = useState(0);
  const toggle = useRef<HTMLSpanElement>(null);
  const shown = searchShown({ asked, text, findShown: header.findShown });
  const open = () => {
    setAsked(true);
    setWantFocus((n) => n + 1);
  };
  useEffect(() => {
    if (p.focusTick) open();
  }, [p.focusTick]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (wantFocus > 0) p.inputRef.current?.focus();
  }, [wantFocus]); // eslint-disable-line react-hooks/exhaustive-deps
  const { apps, load: loadApps } = useCaptureApps();
  const { people, load: loadPeople } = usePeople();
  // A filter belongs to the chips: where they are not shown they do not apply.
  const chips = p.filters || header.findShown;
  const filtered = chips && hasFilter(filter);
  // The Ask control's name is a promise, so it is read the way the Find page reads it: where the filters apply, with the conversation on
  // screen. Only once the AI settings are known; before that it says plain "Ask".
  const readiness = askReadiness(useAskState());
  const aiReady = readiness === "ready";
  const applied = useMemo(() => filterInput(chips ? filter : NO_FILTER, header.filterAt || Date.now()), [chips, filter, header.filterAt]);
  const continued = useContinuedConversation(filterKey(applied));
  const preview = useRoutePreview(text, continued, applied, aiReady);
  const setup = readiness === "unavailable";
  const askName = setup ? t("Set up an AI provider") : askLabel(aiReady ? preview : null);
  const submit = (mode: "search" | "ask") => {
    const now = headerState();
    const query = now.text.trim();
    if (mode === "ask" ? !query : !query && !(chips && hasFilter(now.filter))) return;
    if (mode === "ask" && p.onAsk) {
      // No AI provider yet: the set-up the control itself leads to, as on the Mac. Otherwise the section may answer for itself.
      if (setup) return void openAiSettings();
      // `filtered`: a chip the person can see is set, and a section's own answer takes no filter, so the Find page asks it (it obeys the chips).
      if (p.onAsk(query, aiReady ? preview : null, chips && hasFilter(now.filter))) return;
    }
    if (!chips && hasFilter(now.filter)) setHeaderFilter(NO_FILTER);
    requestFind(mode);
    p.onFind();
  };
  return (
    <div className="w-shell-header">
      <div className="w-shell-header-row" data-search={shown ? "open" : "closed"}>
        <div className="w-shell-title">
          <h1>{p.title}</h1>
          {p.date ? <p>{p.date}</p> : null}
        </div>
        <span className="w-shell-search-toggle" ref={toggle}>
          <WIconButton icon="search" label={t("Search your work memory")} aria-expanded={shown} aria-controls="w-shell-search" aria-keyshortcuts="Control+K" onClick={open} />
        </span>
        <div
          className="w-shell-search"
          id="w-shell-search"
          data-section="composer"
          onKeyDown={(e) => {
            // Escape in an empty field puts it away again (where it is collapsed) and returns to the icon. Escape on the Ask button
            // beside the field does not: focus is not in the field.
            if (escapeCloses({ key: e.key, inField: (e.target as HTMLElement).tagName === "INPUT", text, findShown: header.findShown })) {
              setAsked(false);
              toggle.current?.querySelector("button")?.focus();
            }
          }}
        >
          <WAskBar
            variant={p.ask}
            value={text}
            onChange={setHeaderText}
            onSubmit={() => submit("search")}
            onAltSubmit={() => submit("ask")}
            inputRef={p.inputRef}
            label={t("Search your work memory")}
            placeholder={p.placeholder}
            hint="Ctrl+K"
            submitLabel={t("Search")}
            describedBy={filtered ? "th-filter-summary" : undefined}
            secondary={
              header.findShown ? undefined : (
                <span className="w-shell-ask">
                  <WButton
                    icon="spark"
                    hint={setup ? undefined : "Ctrl+Enter"}
                    aria-label={askName}
                    aria-keyshortcuts={setup ? undefined : "Control+Enter"}
                    title={askName}
                    disabled={!setup && text.trim() === ""}
                    onClick={() => (setup ? void openAiSettings() : submit("ask"))}
                  >
                    {askName}
                  </WButton>
                </span>
              )
            }
          />
        </div>
        <div className="w-shell-capture">{p.capture}</div>
      </div>
      {chips ? (
        <div className="w-shell-filters">
          <div className="w-shell-chips">
            <HomeFilters filter={filter} onChange={setHeaderFilter} apps={apps} onOpenApps={loadApps} people={people} onOpenPeople={loadPeople} now={p.now} />
            {header.findShown && header.count ? (
              <p className="th-filter-count" role="status">
                {header.count}
              </p>
            ) : null}
          </div>
          {filtered ? (
            <p id="th-filter-summary" className="th-filter-summary">
              {filterWords(filter)}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
