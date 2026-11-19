// The Apps and Dates chips under Home's search bar. People is not drawn: captures record no person.
// A chip opens a small menu; choosing a value selects the chip, and its Remove button clears it.

import { useEffect, useRef, useState } from "react";
import type { CaptureApp } from "../../api";
import { t } from "../../l10n";
import { WButton, WFilterChip } from "../../system";
import { appName, dateChoiceLabel, type DateChoice, type SearchFilter } from "./homeModel";

type Open = "people" | "apps" | "dates" | null;

const PRESETS: DateChoice[] = [{ kind: "today" }, { kind: "yesterday" }, { kind: "last7" }];

function ymd(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export interface HomeFiltersProps {
  filter: SearchFilter;
  onChange: (filter: SearchFilter) => void;
  /** The apps seen lately, most captures first; null until asked for. */
  apps: CaptureApp[] | null;
  onOpenApps: () => void;
  /** People from the attendees of recent meetings, and a call that reads more of them when the chip opens. */
  people: string[];
  onOpenPeople: () => void;
  now: number;
}

export function HomeFilters({ filter, onChange, apps, onOpenApps, people, onOpenPeople, now }: HomeFiltersProps) {
  const [open, setOpen] = useState<Open>(null);
  const [from, setFrom] = useState(() => ymd(now - 6 * 86_400_000));
  const [to, setTo] = useState(() => ymd(now));
  const root = useRef<HTMLDivElement>(null);

  // Escape closes the menu; a press outside it does too.
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    const away = (e: MouseEvent) => root.current && !root.current.contains(e.target as Node) && setOpen(null);
    window.addEventListener("keydown", key);
    window.addEventListener("mousedown", away);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener("mousedown", away);
    };
  }, [open]);

  const toggle = (which: Exclude<Open, null>) => {
    if (which === "apps" && open !== "apps") onOpenApps();
    if (which === "people" && open !== "people") onOpenPeople();
    setOpen(open === which ? null : which);
  };
  const pickPerson = (person: string | null) => {
    onChange({ ...filter, person });
    setOpen(null);
  };
  const pickApp = (app: string | null) => {
    onChange({ ...filter, app });
    setOpen(null);
  };
  const pickDates = (dates: DateChoice | null) => {
    onChange({ ...filter, dates });
    setOpen(null);
  };

  return (
    <div className="th-filters" ref={root}>
      <div className="th-chip">
        <WFilterChip
          label={filter.person ? t("People: {name}", { name: filter.person }) : t("People")}
          icon="people"
          menu={!filter.person}
          selected={!!filter.person}
          onPress={() => toggle("people")}
          onRemove={filter.person ? () => pickPerson(null) : undefined}
        />
        {open === "people" ? (
          <div className="th-menu" role="menu" aria-label={t("People")}>
            <button type="button" role="menuitemradio" aria-checked={!filter.person} className="w-menu-item" onClick={() => pickPerson(null)}>
              {t("Anyone")}
            </button>
            {people.map((name) => (
              <button key={name} type="button" role="menuitemradio" aria-checked={filter.person === name} className="w-menu-item" onClick={() => pickPerson(name)}>
                {name}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="th-chip">
        <WFilterChip
          label={filter.app ? t("Apps: {name}", { name: appName(filter.app) }) : t("Apps")}
          icon="apps"
          menu={!filter.app}
          selected={!!filter.app}
          onPress={() => toggle("apps")}
          onRemove={filter.app ? () => pickApp(null) : undefined}
        />
        {open === "apps" ? (
          <div className="th-menu" role="menu" aria-label={t("Apps")}>
            <button type="button" role="menuitemradio" aria-checked={!filter.app} className="w-menu-item" onClick={() => pickApp(null)}>
              {t("Any app")}
            </button>
            {(apps ?? []).map((a) => (
              <button key={a.app_bundle_id} type="button" role="menuitemradio" aria-checked={filter.app === a.app_bundle_id} className="w-menu-item" onClick={() => pickApp(a.app_bundle_id)}>
                {appName(a.app_bundle_id)}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="th-chip">
        <WFilterChip
          label={filter.dates ? t("Dates: {range}", { range: dateChoiceLabel(filter.dates) }) : t("Dates")}
          icon="calendar"
          menu={!filter.dates}
          selected={!!filter.dates}
          onPress={() => toggle("dates")}
          onRemove={filter.dates ? () => pickDates(null) : undefined}
        />
        {open === "dates" ? (
          <div className="th-menu" role="menu" aria-label={t("Dates")}>
            <button type="button" role="menuitemradio" aria-checked={!filter.dates} className="w-menu-item" onClick={() => pickDates(null)}>
              {t("Any date")}
            </button>
            {PRESETS.map((p) => (
              <button key={p.kind} type="button" role="menuitemradio" aria-checked={filter.dates?.kind === p.kind} className="w-menu-item" onClick={() => pickDates(p)}>
                {dateChoiceLabel(p)}
              </button>
            ))}
            <div className="th-range" role="group" aria-label={t("Custom range")}>
              <label>
                {t("From")}
                <input type="date" value={from} max={to} onChange={(e) => setFrom(e.target.value)} />
              </label>
              <label>
                {t("To")}
                <input type="date" value={to} min={from} onChange={(e) => setTo(e.target.value)} />
              </label>
              <WButton disabled={!from || !to || to < from} onClick={() => pickDates({ kind: "custom", from, to })}>
                {t("Apply")}
              </WButton>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
