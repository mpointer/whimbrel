// The header row at the top of the content on every section: the section's title on the left (on Home with the date
// under it), the search and ask bar in the middle, the capture state on the right. On Home the filter chips sit under it.
// Submitting asks, as Today's field did; with a filter chosen it searches Library instead. With no AI provider the
// button is the existing "set up" action, not a dead field.

import { useState, type ReactNode, type RefObject } from "react";
import type { SearchRequest } from "../../library/searchRequest";
import { t } from "../../l10n";
import { WAskBar } from "../../system";
import { HomeFilters } from "../../today/redesign/HomeFilters";
import { appName, dateChoiceLabel, hasFilter, NO_FILTER, searchArgs, type SearchFilter } from "../../today/redesign/homeModel";
import { useAskState, useCaptureApps, usePeople } from "../../today/redesign/useHomeData";
import type { ElementChoices } from "../../today/redesign/elements";

export interface HeaderRowProps {
  title: string;
  /** The date under the title, on Home. */
  date?: string;
  /** The capture state control, drawn on every section once the shell has answered. */
  capture: ReactNode;
  ask: ElementChoices["ask"];
  placeholder: string;
  /** Home: draw the filter chips under the row. */
  filters: boolean;
  now: number;
  inputRef: RefObject<HTMLInputElement>;
  onAsk: (prompt: string) => void;
  onSearch: (request: Omit<SearchRequest, "id">) => void;
  onOpenAiSettings: () => void;
}

export function HeaderRow(p: HeaderRowProps) {
  const [text, setText] = useState("");
  const [filter, setFilter] = useState<SearchFilter>(NO_FILTER);
  const state = useAskState();
  const { apps, load: loadApps } = useCaptureApps();
  const { people, load: loadPeople } = usePeople();
  // A filter belongs to Home's chips; off Home they are not shown and do not apply.
  const filtered = p.filters && hasFilter(filter);
  const aiMissing = !!state && !state.available;
  const summary = [filter.person, filter.app ? appName(filter.app) : null, filter.dates ? dateChoiceLabel(filter.dates) : null].filter(Boolean).join(", ");
  const submit = () => {
    const query = text.trim();
    if (filtered) {
      if (!query) return;
      p.onSearch({ query, ...searchArgs(filter, Date.now()), summary });
    } else if (aiMissing) p.onOpenAiSettings();
    else if (query) p.onAsk(query);
    else return;
    setText("");
  };
  return (
    <div className="w-shell-header">
      <div className="w-shell-header-row">
        <div className="w-shell-title">
          <h1>{p.title}</h1>
          {p.date ? <p>{p.date}</p> : null}
        </div>
        <div className="w-shell-search" data-section="composer">
          <WAskBar
            variant={p.ask}
            value={text}
            onChange={setText}
            onSubmit={submit}
            inputRef={p.inputRef}
            label={t("Ask Whimbrel")}
            placeholder={p.placeholder}
            hint="Ctrl+K"
            submitLabel={filtered ? t("Search") : aiMissing ? t("Set up an AI provider") : t("Ask")}
            describedBy={filtered ? "th-filter-summary" : undefined}
          />
        </div>
        <div className="w-shell-capture">{p.capture}</div>
      </div>
      {p.filters ? (
        <div className="w-shell-filters">
          <HomeFilters filter={filter} onChange={setFilter} apps={apps} onOpenApps={loadApps} people={people} onOpenPeople={loadPeople} now={p.now} />
          {filtered ? (
            <p id="th-filter-summary" className="th-filter-summary">
              {filter.person ? t("Meetings with {name}, and captures that mention {name}.", { name: filter.person }) : t("Searches captures from {filters}.", { filters: summary })}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
