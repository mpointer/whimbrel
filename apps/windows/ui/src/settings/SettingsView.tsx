import { useEffect, useState } from "react";
import { profileSuggestions } from "../api";
import { t } from "../l10n";
import { AboutCard } from "./AboutCard";
import { AccountsCard } from "./AccountsCard";
import { AiCard } from "./AiCard";
import { CaptureCard } from "./CaptureCard";
import { GeneralCard } from "./GeneralCard";
import { GmailCard } from "./GmailCard";
import { GoogleClientCard } from "./GoogleClientCard";
import { HotkeyCard } from "./HotkeyCard";
import { McpCard } from "./McpCard";
import { PlanCard } from "./PlanCard";
import { MeetingModelsCard } from "./MeetingModelsCard";
import { MeetingsCard } from "./MeetingsCard";
import { ProfileCard } from "./ProfileCard";
import { RetentionCard } from "./RetentionCard";
import { StorageCard } from "./StorageCard";
import { EncryptionCard } from "./EncryptionCard";
import { DeleteEverythingCard } from "./DeleteEverythingCard";
import { ForeignFilesCard } from "./ForeignFilesCard";
import { SearchModelCard } from "./SearchModelCard";
import { SETTINGS_SECTIONS, sectionTitle, type SettingsSectionId } from "./settingsSections";
import { searchSettings, type SettingsSearchEntry } from "./settingsSearch";
import { SkinGallery } from "./SkinGallery";
import { StatusPage } from "./StatusPage";
import { SyncCard } from "./SyncCard";
import { TopicsCard } from "./TopicsCard";
import { VaultCard } from "./VaultCard";
import { TopicStepsCard } from "./TopicStepsCard";
import { TypeSafeCard } from "./TypeSafeCard";

/** How often the Profile badge recounts the suggestions waiting for Add or Dismiss. */
const BADGE_REFRESH_MS = 30_000;

/** How long a found setting stays highlighted, and how long to wait for its card to load. */
const HIGHLIGHT_MS = 1500;
const FIND_ROW_MS = 3000;

/**
 * Scrolls the row with this anchor into view and highlights it briefly. Cards load their data first,
 * so it looks again each frame until the row is there (or gives up). Returns a cancel function.
 */
export function revealAnchor(id: string, doc: Document = document): () => void {
  const started = Date.now();
  let frame = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const look = () => {
    const el = doc.querySelector<HTMLElement>(`[data-settings-anchor="${CSS.escape(id)}"]`);
    if (el) {
      el.scrollIntoView({ block: "center", behavior: "smooth" });
      el.classList.add("settings-found");
      timer = setTimeout(() => el.classList.remove("settings-found"), HIGHLIGHT_MS);
      return;
    }
    if (Date.now() - started < FIND_ROW_MS) frame = requestAnimationFrame(look);
  };
  frame = requestAnimationFrame(look);
  return () => {
    cancelAnimationFrame(frame);
    if (timer) clearTimeout(timer);
  };
}

/** Settings: a list of sections on the left and the chosen section's cards on the right (the Mac's layout). */
export function SettingsView(props: {
  section: SettingsSectionId;
  onSection: (id: SettingsSectionId) => void;
  onRetentionSaved: () => void;
  onOpenAutomations: () => void;
}) {
  const { section, onSection, onRetentionSaved, onOpenAutomations } = props;
  const [pending, setPending] = useState(0);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ id: string; n: number } | null>(null);
  const results = searchSettings(query, sectionTitle);

  // After a result opens its section, bring its row into view.
  useEffect(() => (found ? revealAnchor(found.id) : undefined), [found]);

  const choose = (entry: SettingsSearchEntry) => {
    onSection(entry.section);
    setQuery("");
    setFound((f) => ({ id: entry.id, n: (f?.n ?? 0) + 1 }));
  };

  useEffect(() => {
    const count = () => profileSuggestions().then((list) => setPending(list.length), () => {});
    count();
    const timer = setInterval(count, BADGE_REFRESH_MS);
    return () => clearInterval(timer);
  }, [section]);

  return (
    <div className="settings-layout">
      <nav className="settings-nav" aria-label={t("Settings")}>
        <input
          type="search"
          className="input settings-search"
          placeholder={t("Search settings")}
          aria-label={t("Search settings")}
          value={query}
          onChange={(ev) => setQuery(ev.target.value)}
          onKeyDown={(ev) => {
            if (ev.key === "Enter" && results[0]) choose(results[0]);
            if (ev.key === "Escape") setQuery("");
          }}
        />
        {query.trim() ? <SearchResults results={results} onChoose={choose} /> : null}
        <ul hidden={!!query.trim()}>
          {SETTINGS_SECTIONS.map((id) => (
            <li key={id}>
              <button
                id={`settings-${id}`}
                className={id === section ? "settings-nav-item settings-nav-item--active" : "settings-nav-item"}
                aria-current={id === section ? "page" : undefined}
                onClick={() => onSection(id)}
              >
                {sectionTitle(id)}
                {id === "profile" && pending > 0 && <span className="tab-badge">{pending}</span>}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <div className="settings-panel stack" aria-labelledby={`settings-${section}`}>
        {section === "status" ? (
          <StatusPage onSection={onSection} onOpenAutomations={onOpenAutomations} />
        ) : (
          <SectionCards section={section} onSection={onSection} onRetentionSaved={onRetentionSaved} />
        )}
      </div>
    </div>
  );
}

export function SearchResults({ results, onChoose }: { results: SettingsSearchEntry[]; onChoose: (e: SettingsSearchEntry) => void }) {
  if (results.length === 0) return <p className="muted small settings-search-empty">{t("No matching settings")}</p>;
  return (
    <ul className="settings-search-results" aria-label={t("Search settings")}>
      {results.map((r) => (
        <li key={r.id}>
          <button type="button" className="settings-nav-item" onClick={() => onChoose(r)}>
            <span>{r.title}</span>
            <span className="muted small"> · {sectionTitle(r.section)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function SectionCards({
  section,
  onSection,
  onRetentionSaved,
}: {
  section: Exclude<SettingsSectionId, "status">;
  onSection: (id: SettingsSectionId) => void;
  onRetentionSaved: () => void;
}) {
  switch (section) {
    case "general":
      return (
        <>
          <GeneralCard />
          <SkinGallery />
          <AboutCard />
        </>
      );
    case "profile":
      return <ProfileCard />;
    case "plan":
      return <PlanCard onSection={onSection} />;
    case "capture":
      return (
        <>
          <CaptureCard onOpenAi={() => onSection("ai")} />
          <HotkeyCard />
          <RetentionCard onSaved={onRetentionSaved} />
          <StorageCard />
          <EncryptionCard />
          <ForeignFilesCard />
          <VaultCard />
          <DeleteEverythingCard />
        </>
      );
    case "meetings":
      return (
        <>
          <MeetingsCard />
          <MeetingModelsCard />
        </>
      );
    case "ai":
      return (
        <>
          <AiCard />
          <TypeSafeCard />
          <SearchModelCard />
        </>
      );
    case "topics":
      return (
        <>
          <TopicsCard />
          <TopicStepsCard />
        </>
      );
    case "devices":
      return <SyncCard />;
    case "connectors":
      return (
        <>
          <AccountsCard />
          <GmailCard />
          <GoogleClientCard />
          <McpCard />
        </>
      );
  }
}
