// The redesigned Settings tab (docs/ui-redesign/screens/settings.md), drawn only with the hidden redesign switch on.
// The frame: a search field and the section list on the left (a labelled picker in a narrow window or at large text),
// the open section on the right, the whole group centred with a maximum width as Today and Find are. The sections
// are in ./sections; each reads and writes through the same api.ts commands as the old cards.
import { useEffect, useRef, useState } from "react";
import { profileSuggestions } from "../../api";
import { t } from "../../l10n";
import { WSettingsLayout, WSettingsNav, WSettingsSearchResults, type WSettingsNavItem } from "../../system";
import { SETTINGS_SEARCH_ENTRIES, type SettingsSearchEntry } from "../settingsSearch";
import type { SettingsSectionId } from "../settingsSections";
import { JumpContext } from "./jump";
import { REDESIGN_SECTIONS, redesignSectionTitle, sectionPickerLabel } from "./sections";
import { searchRedesign } from "./search";
import { ConnectorsSection } from "./sections/ConnectorsSection";
import { McpTokenHost } from "./sections/connectors/McpTokenHost";
import { DevicesSection } from "./sections/DevicesSection";
import { AiSection } from "./sections/AiSection";
import { GeneralSection } from "./sections/GeneralSection";
import { MeetingsSection } from "./sections/MeetingsSection";
import { MemorySection } from "./sections/MemorySection";
import { PlanSection } from "./sections/PlanSection";
import { ProfileSection } from "./sections/ProfileSection";
import { StatusSection } from "./sections/StatusSection";
import { TopicsSection } from "./sections/TopicsSection";
import type { SectionProps } from "./types";
import "./settings-redesign.css";

/** How often the Profile badge recounts the suggestions waiting for Add or Dismiss (as the old tab does). */
const BADGE_REFRESH_MS = 30_000;
/** How long the mark stays on a row a search result opened. */
const JUMP_MS = 2500;
/** How long to wait for a card to load before giving up on finding the row. */
const FIND_MS = 3000;

export function SectionBody({ section, ...props }: SectionProps & { section: SettingsSectionId }) {
  switch (section) {
    case "status":
      return <StatusSection {...props} />;
    case "general":
      return <GeneralSection {...props} />;
    case "profile":
      return <ProfileSection {...props} />;
    case "plan":
      return <PlanSection {...props} />;
    case "capture":
      return <MemorySection {...props} />;
    case "meetings":
      return <MeetingsSection {...props} />;
    case "ai":
      return <AiSection {...props} />;
    case "topics":
      return <TopicsSection {...props} />;
    case "devices":
      return <DevicesSection {...props} />;
    case "connectors":
      return <ConnectorsSection {...props} />;
  }
}

/** The ids the search can open, so a jump to something the registry does not know is ignored. */
const KNOWN_ANCHORS = new Set(SETTINGS_SEARCH_ENTRIES.map((e) => e.id));

/** Keeps `data-settings-anchor` equal to the element's id wherever that id is a search anchor, and nowhere else: a node
 *  React reuses for another row (after a state change) loses the stamp it carried for the row it used to be. Idempotent. */
export function stampAnchors(root: ParentNode): void {
  for (const el of root.querySelectorAll<HTMLElement>("[data-settings-anchor]")) {
    const id = el.getAttribute("id");
    const want = id !== null && KNOWN_ANCHORS.has(id) ? id : null;
    if (want === null) el.removeAttribute("data-settings-anchor");
    else if (el.getAttribute("data-settings-anchor") !== want) el.setAttribute("data-settings-anchor", want);
  }
  for (const id of KNOWN_ANCHORS) {
    const el = root.querySelector<HTMLElement>(`[id="${id}"]`);
    if (el && el.getAttribute("data-settings-anchor") !== id) el.setAttribute("data-settings-anchor", id);
  }
}

export function RedesignedSettings(props: {
  section: SettingsSectionId;
  onSection: (id: SettingsSectionId) => void;
  onRetentionSaved: () => void;
  onOpenAutomations: () => void;
}) {
  const { section, onSection, onRetentionSaved, onOpenAutomations } = props;
  const [pending, setPending] = useState(0);
  const [query, setQuery] = useState("");
  const [jump, setJump] = useState<string | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const first = useRef(true);
  const results = searchRedesign(query);
  const searching = query.trim() !== "";

  useEffect(() => {
    const count = () => profileSuggestions().then((list) => setPending(list.length), () => {});
    count();
    const timer = setInterval(count, BADGE_REFRESH_MS);
    return () => clearInterval(timer);
  }, [section]);

  // Choosing a section moves focus to its heading, so a screen reader starts there. Not on the first draw, and not
  // when a search result is about to move focus to its own row.
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    // Arrowing the picker (a select in a narrow window) must not pull focus away from it.
    if (!jump && document.activeElement?.tagName !== "SELECT") heading.current?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [section]);

  // A search result: open its section, then bring the row (or the card) into view. Cards load first, so look again
  // each frame until it is there. A row marks itself through `useJump`; a card gets focus on its heading. The mark is
  // kept for JUMP_MS from the moment the row first exists, not from the click, so a slow card does not lose it.
  useEffect(() => {
    if (!jump) return;
    const started = Date.now();
    let frame = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const look = () => {
      const el = document.getElementById(jump);
      if (el) {
        if (el.classList.contains("w-scard")) {
          const h = el.querySelector<HTMLElement>("h2, h3, h4");
          h?.setAttribute("tabindex", "-1");
          h?.focus({ preventScroll: true });
          el.scrollIntoView?.({ block: "start" });
        }
        timer = setTimeout(() => setJump(null), JUMP_MS);
        return;
      }
      if (Date.now() - started < FIND_MS) frame = requestAnimationFrame(look);
      else setJump(null);
    };
    frame = requestAnimationFrame(look);
    return () => {
      cancelAnimationFrame(frame);
      if (timer) clearTimeout(timer);
    };
  }, [jump]);

  // The parts take an `id` for a jump; the reorganisation's tests and search read `data-settings-anchor`. The frame writes
  // that attribute on every element whose id is a registry anchor, now and as cards load, so a row carries the same
  // value in both tabs.
  const content = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = content.current;
    if (!root) return;
    const stamp = () => stampAnchors(root);
    stamp();
    const watch = new MutationObserver(stamp);
    watch.observe(root, { childList: true, subtree: true });
    return () => watch.disconnect();
  }, [section]);

  const open = (id: string) => {
    const entry: SettingsSearchEntry | undefined = results.find((r) => r.id === id);
    if (!entry || !KNOWN_ANCHORS.has(id)) return;
    onSection(entry.section);
    setQuery("");
    setJump(null);
    // A new value even for the same row, so a second search for it marks it again.
    setTimeout(() => setJump(id), 0);
  };

  const items: WSettingsNavItem[] = REDESIGN_SECTIONS.map((id) => ({
    id,
    label: redesignSectionTitle(id),
    ...(id === "profile" && pending > 0 ? { badge: pending, badgeLabel: t("{n} suggestions waiting", { n: String(pending) }) } : {}),
  }));

  const nav = (
    <>
      <input
        type="search"
        className="w-input rs-search"
        placeholder={t("Search settings")}
        aria-label={t("Search settings")}
        value={query}
        onChange={(ev) => setQuery(ev.target.value)}
        onKeyDown={(ev) => {
          if (ev.key === "Enter" && results[0]) open(results[0].id);
          if (ev.key === "Escape") setQuery("");
        }}
      />
      {searching ? (
        <WSettingsSearchResults
          results={results.map((r) => ({ id: r.id, title: r.title, section: redesignSectionTitle(r.section) }))}
          onOpen={open}
          summary={t("{n} settings found", { n: String(results.length) })}
          label={t("Search settings")}
        />
      ) : (
        <WSettingsNav label={t("Settings")} selectLabel={sectionPickerLabel()} items={items} selected={section} onSelect={(id) => onSection(id as SettingsSectionId)} />
      )}
    </>
  );

  return (
    <div className="rs-frame">
      <WSettingsLayout nav={nav}>
        <div className="rs-content" data-settings-section={section} ref={content}>
          <h2 className="rs-title" tabIndex={-1} ref={heading} id={`settings-${section}-title`}>
            {redesignSectionTitle(section)}
          </h2>
          {/* Above the section switch: a new MCP token's dialog (and the notice after a token nobody copied) must outlive a change of section. */}
          <McpTokenHost />
          <JumpContext.Provider value={jump}>
            <SectionBody section={section} onSection={onSection} onRetentionSaved={onRetentionSaved} onOpenAutomations={onOpenAutomations} />
          </JumpContext.Provider>
        </div>
      </WSettingsLayout>
    </div>
  );
}
