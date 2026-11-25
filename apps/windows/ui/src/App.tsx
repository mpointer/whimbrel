import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { approvalsCount, takePendingOpenMeeting, takePendingOpenTopic } from "./api";
import { RetentionNotice } from "./settings/RetentionNotice";
import { UpdateBanner } from "./settings/UpdateBanner";
import { ArmNotice } from "./settings/ArmNotice";
import { SignInBanner } from "./settings/SignInBanner";
import { PlusAiPrompt } from "./settings/PlusAiPrompt";
import { SyncStoppedLine } from "./settings/SyncStoppedLine";
import { bootstrapModels } from "./settings/modelBootstrap";
import { ModelStatusLine } from "./settings/ModelStatusLine";
import { SettingsView } from "./settings/SettingsView";
import { RedesignedSettings } from "./settings/redesign/RedesignedSettings";
import { DEFAULT_SECTION, toSection, type SettingsSectionId } from "./settings/settingsSections";
import { AutomationsView } from "./automations/AutomationsView";
import { waitingLabel } from "./automations/automationsModel";
import type { AskRequest } from "./chat/chatModel";
import { LibraryView } from "./library/LibraryView";
import { MeetingsView } from "./meetings/MeetingsView";
import { TodayView } from "./today/TodayView";
import { DEFAULT_TAB, shownTab, tabs, type TabId } from "./tabs";
import { t } from "./l10n";
import { SkinHeader, TabIcon } from "./SkinIcon";
import { TopicsView } from "./topics/TopicsView";
import { OPEN_TOPIC_EVENT } from "./topics/TopicChip";
import { useNoStore } from "./launch/noStore";
import { NoStoreBanner } from "./launch/NoStoreBanner";
import { LibraryMissingBanner } from "./settings/LibraryMissingBanner";
import { useRedesign } from "./redesign";
import { RedesignedShell } from "./shell/redesign/RedesignedShell";
import type { SearchRequest } from "./library/searchRequest";
import { FindLibrary } from "./library/redesign/FindLibrary";
import { requestFindWith } from "./library/redesign/findStore";
import { ConversationsView } from "./conversations/ConversationsView";
import { askTarget } from "./conversations/model";
import { startConversation, topicShownNow, useConversationNav } from "./conversations/nav";
import { askOnToday } from "./conversations/todayStore";
import type { RoutePreviewView } from "./generated/types";
import "./decoration.css";

export function App() {
  const [active, setActive] = useState<TabId>(DEFAULT_TAB);
  const noStore = useNoStore();
  // Bumped when retention windows are saved, so the first-run notice recounts against
  // the new windows: what it offers to delete is always what it is showing.
  const [retentionSaved, setRetentionSaved] = useState(0);
  // How many routine actions wait for approval, for the badge on the Automations tab. The tab itself
  // reports a new count whenever it changes; this keeps it fresh while another tab is showing.
  const [waiting, setWaiting] = useState(0);
  // A question Today's composer hands to Library > Ask; Ask sends it once and clears it.
  const [askRequest, setAskRequest] = useState<AskRequest | null>(null);
  // A filtered search the redesigned Home hands to Library; Library runs it once and clears it.
  const [searchRequest, setSearchRequest] = useState<SearchRequest | null>(null);
  // Which Settings section shows: Status when the tab is chosen (the Mac's issue 595), or the one a
  // button elsewhere asked for.
  const [settingsSection, setSettingsSection] = useState<SettingsSectionId>(DEFAULT_SECTION);
  const openSettings = (id: SettingsSectionId) => {
    setSettingsSection(id);
    setActive("settings");
  };
  const chooseTab = (id: TabId) => {
    if (id === "settings" && active !== "settings") setSettingsSection(DEFAULT_SECTION);
    setActive(id);
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
      setSettingsSection(toSection(event.payload));
      setActive("settings");
    });
    return () => void unlisten.then((stop) => stop());
  }, []);
  // A question typed in the in-meeting window (redesign only; nothing emits it otherwise): Today, where the live screen is.
  useEffect(() => {
    const unlisten = listen("meeting-bar-ask-pending", () => setActive("today"));
    return () => void unlisten.then((stop) => stop());
  }, []);
  // A topic one of the ways in asked for (Today, a meeting, the prep brief, search, the tray).
  const [topicRequest, setTopicRequest] = useState<{ seq: number; topicId: number } | null>(null);
  const openTopic = (topicId: number) => {
    setTopicRequest((r) => ({ seq: (r?.seq ?? 0) + 1, topicId }));
    setActive("topics");
  };
  useEffect(() => {
    const unlisten = listen<number>(OPEN_TOPIC_EVENT, (event) => openTopic(event.payload));
    return () => void unlisten.then((stop) => stop());
  }, []);
  // A topic the tray popup asked for: read on mount (opening it may have just created this window)
  // and whenever the popup says one is waiting.
  useEffect(() => {
    const take = () =>
      takePendingOpenTopic().then((id) => {
        if (id != null) openTopic(id);
      }, () => {});
    take();
    const unlisten = listen("open-topic-pending", take);
    return () => void unlisten.then((stop) => stop());
  }, []);
  // Missing models download in the background, with no prompt (Release 1 item 1.1).
  useEffect(bootstrapModels, []);
  // Something asked for the Conversations section (Today's Continue, a meeting answer's Continue, a topic's Ask, Find opening a topic or meeting
  // conversation): show it. Nothing asks while the switch is off.
  const conversationRequest = useConversationNav().request;
  useEffect(() => {
    if (conversationRequest > 0) setActive("conversations");
  }, [conversationRequest]);
  // The hidden redesign switch, asked after every hook above: with it off, nothing below differs from before.
  const redesign = useRedesign();
  const allTabs = tabs(redesign);
  // A section the switch hides (Conversations, with the switch turned off while it was open) falls back to the default one.
  const tab = shownTab(allTabs, active);

  const content = (
    <>
      {noStore && <NoStoreBanner onOpenSettings={() => openSettings("capture")} />}
      <LibraryMissingBanner />
      <UpdateBanner />
      <ArmNotice />
      <SignInBanner />
      <RetentionNotice key={retentionSaved} onChangeSettings={() => openSettings("capture")} />
      <PlusAiPrompt />
      {/* Today writes its own heading: the greeting. The redesigned shell's header row carries every title. */}
      {!redesign && tab.id !== "today" && <h1 className="display">{tab.label}</h1>}
      {tab.id === "settings" ? (
        redesign ? (
          <RedesignedSettings
            section={settingsSection}
            onSection={setSettingsSection}
            onRetentionSaved={() => setRetentionSaved((n) => n + 1)}
            onOpenAutomations={() => setActive("automations")}
          />
        ) : (
          <SettingsView
            section={settingsSection}
            onSection={setSettingsSection}
            onRetentionSaved={() => setRetentionSaved((n) => n + 1)}
            onOpenAutomations={() => setActive("automations")}
          />
        )
      ) : tab.id === "today" ? (
        <TodayView
          onAsk={(prompt) => {
            if (redesign) {
              // The redesigned Library answers on its Find page, from the field's words.
              requestFindWith(prompt, "ask");
            } else setAskRequest((r) => ({ id: (r?.id ?? 0) + 1, prompt }));
            setActive("library");
          }}
          onOpenSettings={() => openSettings("ai")}
          onOpenSection={(target) => (target === "automations" ? setActive("automations") : target === "library" ? setActive("library") : openSettings(target))}
        />
      ) : tab.id === "meetings" ? (
        <MeetingsView open={openMeeting} onOpenSettings={() => openSettings("ai")} />
      ) : tab.id === "library" && redesign ? (
        <FindLibrary onOpenSettings={() => openSettings("ai")} />
      ) : tab.id === "library" ? (
        <LibraryView
          askRequest={askRequest}
          onAskHandled={() => setAskRequest(null)}
          searchRequest={searchRequest}
          onSearchHandled={() => setSearchRequest(null)}
          onOpenSettings={() => openSettings("ai")}
        />
      ) : tab.id === "topics" ? (
        <TopicsView request={topicRequest} />
      ) : tab.id === "conversations" && redesign ? (
        <ConversationsView />
      ) : tab.id === "automations" ? (
        <AutomationsView onWaiting={setWaiting} />
      ) : (
        <div className="card empty-state">
          {tab.comingIn ? t("Coming in {milestone}.", { milestone: tab.comingIn }) : t("Nothing here yet — this tab fills in during M3.2.")}
        </div>
      )}
    </>
  );

  /** The shell's Ask control, screen 6 (the Mac's `WhimbrelTab.askTarget`): on Today the answer shows under the bar; on Conversations or on a topic's
   *  page a new conversation asks about everything or that topic; anywhere else the Find page's own Ask, as before (false). Enter always searches. */
  const askFromHeader = (query: string, shown: RoutePreviewView | null, filtered: boolean): boolean => {
    const topic = topicShownNow();
    const target = askTarget(tab.id, topic?.id ?? null, filtered);
    if (target.kind === "library") return false;
    if (target.kind === "today") void askOnToday(query, shown);
    else startConversation(target.scope, query, target.scope.kind === "topic" ? (topic?.name ?? null) : null, shown);
    return true;
  };

  if (redesign) {
    return (
      <RedesignedShell
        tabs={allTabs}
        active={tab.id}
        onChoose={chooseTab}
        waiting={waiting}
        onWhatIsCaptured={() => openSettings("capture")}
        onOpenStatus={() => openSettings("status")}
        onFind={() => setActive("library")}
        onAsk={askFromHeader}
      >
        {content}
      </RedesignedShell>
    );
  }

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
                aria-selected={each.id === tab.id}
                aria-controls="panel"
                className={each.id === tab.id ? "tab tab--active" : "tab"}
                onClick={() => chooseTab(each.id)}
              >
                <TabIcon id={each.id} />
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
        <SyncStoppedLine onOpen={() => openSettings("status")} />
      </nav>
      <div className="stage">
      <SkinHeader />
      <main id="panel" role="tabpanel" aria-labelledby={`tab-${tab.id}`} className="content">
        {content}
      </main>
      </div>
    </div>
  );
}
