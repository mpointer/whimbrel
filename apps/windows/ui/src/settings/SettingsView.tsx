import { useEffect, useState } from "react";
import { profileSuggestions } from "../api";
import { t } from "../l10n";
import { AboutCard } from "./AboutCard";
import { AccountsCard } from "./AccountsCard";
import { AiCard } from "./AiCard";
import { CaptureCard } from "./CaptureCard";
import { GeneralCard } from "./GeneralCard";
import { GmailCard } from "./GmailCard";
import { McpCard } from "./McpCard";
import { MeetingModelsCard } from "./MeetingModelsCard";
import { MeetingsCard } from "./MeetingsCard";
import { ProfileCard } from "./ProfileCard";
import { RetentionCard } from "./RetentionCard";
import { SearchModelCard } from "./SearchModelCard";
import { SETTINGS_SECTIONS, sectionTitle, type SettingsSectionId } from "./settingsSections";
import { SkinGallery } from "./SkinGallery";
import { StatusPage } from "./StatusPage";
import { SyncCard } from "./SyncCard";

/** How often the Profile badge recounts the suggestions waiting for Add or Dismiss. */
const BADGE_REFRESH_MS = 30_000;

/** Settings: a list of sections on the left and the chosen section's cards on the right (the Mac's layout). */
export function SettingsView(props: {
  section: SettingsSectionId;
  onSection: (id: SettingsSectionId) => void;
  onRetentionSaved: () => void;
  onOpenAutomations: () => void;
}) {
  const { section, onSection, onRetentionSaved, onOpenAutomations } = props;
  const [pending, setPending] = useState(0);

  useEffect(() => {
    const count = () => profileSuggestions().then((list) => setPending(list.length), () => {});
    count();
    const timer = setInterval(count, BADGE_REFRESH_MS);
    return () => clearInterval(timer);
  }, [section]);

  return (
    <div className="settings-layout">
      <nav className="settings-nav" aria-label={t("Settings")}>
        <ul>
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
          <SectionCards section={section} onRetentionSaved={onRetentionSaved} />
        )}
      </div>
    </div>
  );
}

function SectionCards({ section, onRetentionSaved }: { section: Exclude<SettingsSectionId, "status">; onRetentionSaved: () => void }) {
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
    case "capture":
      return (
        <>
          <CaptureCard />
          <RetentionCard onSaved={onRetentionSaved} />
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
          <SearchModelCard />
        </>
      );
    case "devices":
      return <SyncCard />;
    case "connectors":
      return (
        <>
          <AccountsCard />
          <GmailCard />
          <McpCard />
        </>
      );
  }
}
