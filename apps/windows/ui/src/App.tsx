import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { approvalsCount, takePendingOpenMeeting } from "./api";
import { RetentionNotice } from "./settings/RetentionNotice";
import { UpdateBanner } from "./settings/UpdateBanner";
import { bootstrapModels } from "./settings/modelBootstrap";
import { ModelStatusLine } from "./settings/ModelStatusLine";
import { SettingsView } from "./settings/SettingsView";
import { rememberedSection, rememberSection, toSection, type SettingsSectionId } from "./settings/settingsSections";
import { AutomationsView } from "./automations/AutomationsView";
import { waitingLabel } from "./automations/automationsModel";
import type { AskRequest } from "./chat/chatModel";
import { LibraryView } from "./library/LibraryView";
import { MeetingsView } from "./meetings/MeetingsView";
import { TodayView } from "./today/TodayView";
import { DEFAULT_TAB, tabs, type TabId } from "./tabs";
import { t } from "./l10n";
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
  // Which Settings section shows: the last one chosen, or the one a button elsewhere asked for.
  const [settingsSection, setSettingsSection] = useState<SettingsSectionId>(() => rememberedSection());
  const chooseSection = (id: SettingsSectionId) => {
    setSettingsSection(id);
    rememberSection(id);
  };
  const openSettings = (id: SettingsSectionId) => {
    chooseSection(id);
    setActive("settings");
  };
  useEffect(() => {
    const look = () => approvalsCount().then(setWaiting, () => {});
    look();
    const timer = setInterval(look, 20_000);
    return () => clearInterval(timer);
  }, []);
  // A meeting a question alert asked to open (parity item a). Read on mount, because opening it may
  // have just created this window, and again whenever the alert pings an open window.
  const [openMeeting, setOpenMeeting] = useState<{ seq: number; meetingId: number | null } | null>(null);
  useEffect(() => {
    const take = () =>
      takePendingOpenMeeting().then((request) => {
        if (!request) return;
        setOpenMeeting((o) => ({ seq: (o?.seq ?? 0) + 1, meetingId: request.meetingId ?? null }));
        setActive("meetings");
      }, () => {});
    take();
    const unlisten = listen("open-meeting", take);
    return () => void unlisten.then((stop) => stop());
  }, []);
  // "Raise budget" and "Open AI settings" (provider budgets): Settings, at the section named.
  useEffect(() => {
    const unlisten = listen<string>("open-settings", (event) => {
      const id = toSection(event.payload);
      setSettingsSection(id);
      rememberSection(id);
      setActive("settings");
    });
    return () => void unlisten.then((stop) => stop());
  }, []);
  // Missing models download in the background, with no prompt (Release 1 item 1.1).
  useEffect(bootstrapModels, []);
  const allTabs = tabs();
  const tab = allTabs.find((each) => each.id === active)!;

  return (
    <div className="shell">
      <nav className="sidebar" aria-label={t("Sections")}>
        <div className="brand">{"Whimbrel"}</div>
        <ul role="tablist" aria-orientation="vertical">
          {allTabs.map((each) => (
            <li key={each.id}>
              <button
                role="tab"
                id={`tab-${each.id}`}
                aria-selected={each.id === active}
                aria-controls="panel"
                className={each.id === active ? "tab tab--active" : "tab"}
                onClick={() => setActive(each.id)}
              >
                <SkinIcon role={TAB_ICON[each.id]} />
                {each.label}
                {each.id === "automations" && waiting > 0 && (
                  <span className="tab-badge" role="status" aria-label={waitingLabel(waiting)}>
                    {waiting}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
        <ModelStatusLine />
      </nav>
      <div className="stage">
      <SkinHeader />
      <main id="panel" role="tabpanel" aria-labelledby={`tab-${tab.id}`} className="content">
        <UpdateBanner />
        <RetentionNotice key={retentionSaved} onChangeSettings={() => openSettings("capture")} />
        {/* Today writes its own heading: the greeting. */}
        {tab.id !== "today" && <h1 className="display">{tab.label}</h1>}
        {tab.id === "settings" ? (
          <SettingsView section={settingsSection} onSection={chooseSection} onRetentionSaved={() => setRetentionSaved((n) => n + 1)} />
        ) : tab.id === "today" ? (
          <TodayView
            onAsk={(prompt) => {
              setAskRequest((r) => ({ id: (r?.id ?? 0) + 1, prompt }));
              setActive("library");
            }}
            onOpenSettings={() => openSettings("ai")}
          />
        ) : tab.id === "meetings" ? (
          <MeetingsView open={openMeeting} />
        ) : tab.id === "library" ? (
          <LibraryView askRequest={askRequest} onAskHandled={() => setAskRequest(null)} onOpenSettings={() => openSettings("ai")} />
        ) : tab.id === "automations" ? (
          <AutomationsView onWaiting={setWaiting} />
        ) : (
          <div className="card empty-state">
            {tab.comingIn ? t("Coming in {milestone}.", { milestone: tab.comingIn }) : t("Nothing here yet — this tab fills in during M3.2.")}
          </div>
        )}
      </main>
      </div>
    </div>
  );
}
