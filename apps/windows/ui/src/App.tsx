import { useEffect, useState } from "react";
import { approvalsCount } from "./api";
import { AboutCard } from "./settings/AboutCard";
import { AiCard } from "./settings/AiCard";
import { CaptureCard } from "./settings/CaptureCard";
import { GeneralCard } from "./settings/GeneralCard";
import { MeetingModelsCard } from "./settings/MeetingModelsCard";
import { AccountsCard } from "./settings/AccountsCard";
import { GmailCard } from "./settings/GmailCard";
import { McpCard } from "./settings/McpCard";
import { RetentionCard } from "./settings/RetentionCard";
import { RetentionNotice } from "./settings/RetentionNotice";
import { UpdateBanner } from "./settings/UpdateBanner";
import { MeetingsCard } from "./settings/MeetingsCard";
import { SearchModelCard } from "./settings/SearchModelCard";
import { SyncCard } from "./settings/SyncCard";
import { ProfileCard } from "./settings/ProfileCard";
import { SkinGallery } from "./settings/SkinGallery";
import { AutomationsView } from "./automations/AutomationsView";
import { waitingLabel } from "./automations/automationsModel";
import type { AskRequest } from "./chat/chatModel";
import { LibraryView } from "./library/LibraryView";
import { MeetingsView } from "./meetings/MeetingsView";
import { TodayView } from "./today/TodayView";
import { DEFAULT_TAB, TABS, type TabId } from "./tabs";
import { SkinHeader, SkinIcon, TAB_ICON } from "./SkinIcon";
import "./decoration.css";

export function App() {
  const [active, setActive] = useState<TabId>(DEFAULT_TAB);
  // Bumped when retention windows are saved, so the first-run notice recounts against
  // the new windows: what it offers to delete is always what it is showing.
  const [retentionSaved, setRetentionSaved] = useState(0);
  // How many routine actions wait for approval, for the badge on the Automations tab. The tab itself
  // reports a new count whenever it changes; this keeps it fresh while another tab is showing.
  const [waiting, setWaiting] = useState(0);
  // A question Today's composer hands to Library > Ask; Ask sends it once and clears it.
  const [askRequest, setAskRequest] = useState<AskRequest | null>(null);
  useEffect(() => {
    const look = () => approvalsCount().then(setWaiting, () => {});
    look();
    const timer = setInterval(look, 20_000);
    return () => clearInterval(timer);
  }, []);
  const tab = TABS.find((t) => t.id === active)!;

  return (
    <div className="shell">
      <nav className="sidebar" aria-label="Sections">
        <div className="brand">Whimbrel</div>
        <ul role="tablist" aria-orientation="vertical">
          {TABS.map((t) => (
            <li key={t.id}>
              <button
                role="tab"
                id={`tab-${t.id}`}
                aria-selected={t.id === active}
                aria-controls="panel"
                className={t.id === active ? "tab tab--active" : "tab"}
                onClick={() => setActive(t.id)}
              >
                <SkinIcon role={TAB_ICON[t.id]} />
                {t.label}
                {t.id === "automations" && waiting > 0 && (
                  <span className="tab-badge" role="status" aria-label={waitingLabel(waiting)}>
                    {waiting}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <div className="stage">
      <SkinHeader />
      <main id="panel" role="tabpanel" aria-labelledby={`tab-${tab.id}`} className="content">
        <UpdateBanner />
        <RetentionNotice key={retentionSaved} onChangeSettings={() => setActive("settings")} />
        {/* Today writes its own heading: the greeting. */}
        {tab.id !== "today" && <h1 className="display">{tab.label}</h1>}
        {tab.id === "settings" ? (
          <div className="stack">
            <GeneralCard />
            <SkinGallery />
            <ProfileCard />
            <SyncCard />
            <AiCard />
            <CaptureCard />
            <MeetingsCard />
            <AccountsCard />
            <GmailCard />
            <McpCard />
            <RetentionCard onSaved={() => setRetentionSaved((n) => n + 1)} />
            <SearchModelCard />
            <MeetingModelsCard />
            <AboutCard />
          </div>
        ) : tab.id === "today" ? (
          <TodayView
            onAsk={(prompt) => {
              setAskRequest((r) => ({ id: (r?.id ?? 0) + 1, prompt }));
              setActive("library");
            }}
            onOpenSettings={() => setActive("settings")}
          />
        ) : tab.id === "meetings" ? (
          <MeetingsView />
        ) : tab.id === "library" ? (
          <LibraryView askRequest={askRequest} onAskHandled={() => setAskRequest(null)} />
        ) : tab.id === "automations" ? (
          <AutomationsView onWaiting={setWaiting} />
        ) : (
          <div className="card empty-state">
            {tab.comingIn ? `Coming in ${tab.comingIn}.` : "Nothing here yet — this tab fills in during M3.2."}
          </div>
        )}
      </main>
      </div>
    </div>
  );
}
