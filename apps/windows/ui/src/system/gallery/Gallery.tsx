import { useEffect, useState, type ReactNode } from "react";
import {
  WActionItemRow,
  WButton,
  WCaptureState,
  WCheckbox,
  WConsentBanner,
  WDialog,
  WEmptyState,
  WErrorState,
  WFilterChip,
  WIcon,
  W_ICON_NAMES,
  WLoadingState,
  WMeetingCard,
  WMemoryControl,
  WRecordingRow,
  WWindowFrame,
  WNextMeeting,
  WSectionLabel,
  WAskBar,
  WSpineNow,
  WMoreMenu,
  WMemoryBlock,
  WNoteBlock,
  WNotice,
  WNumberedList,
  WPanel,
  WProgressLine,
  WSegmented,
  WTextField,
  WLevelMeter,
  WNowMarker,
  WRecordingIndicator,
  WRoot,
  WSearchBar,
  WSettingsRow,
  WSidebar,
  WSidebarItem,
  WSourceCard,
  WSourceMark,
  WSpeakerChip,
  WSpine,
  WSpineStop,
  WTranscriptLine,
  WTranscriptView,
} from "..";
import "./gallery.css";
import { SettingsGallery } from "./SettingsGallery";
import { MEMORY_ITEMS, NOTE, PAUSE_CHOICES, PEOPLE, SOURCES, TRANSCRIPT } from "./sample";

// Debug screen: every component in every state, on the Thread palette. The mode and the text size are set from the
// switchers or from the address (?mode=dark&size=150), which is how the screenshots are taken.
// Labels here are plain strings in expressions, not t(): this screen is not shipped, so none of it goes to a catalog.

const MODES = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
  ["hc-light", "High contrast light"],
  ["hc-dark", "High contrast dark"],
] as const;
const SIZES = [100, 150, 200] as const;
type Mode = (typeof MODES)[number][0];

function fromAddress(): { mode: Mode; size: number; only: string | null } {
  const q = new URLSearchParams(window.location.search);
  const mode = MODES.find(([id]) => id === q.get("mode"))?.[0] ?? "system";
  const size = SIZES.find((s) => String(s) === q.get("size")) ?? 100;
  return { mode, size, only: q.get("only") };
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="g-section">
      <h2 className="g-h2">{title}</h2>
      <div className="g-body">{children}</div>
    </section>
  );
}
function Case({ label, children, wide }: { label: string; children: ReactNode; wide?: boolean }) {
  return (
    <div className={wide ? "g-case g-wide" : "g-case"}>
      <div className="g-label">{label}</div>
      {children}
    </div>
  );
}

const noop = () => {};

export function Gallery() {
  const init = fromAddress();
  const [mode, setMode] = useState<Mode>(init.mode);
  const [size, setSize] = useState<number>(init.size);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<number | null>(3);
  const [chips, setChips] = useState({ people: true, apps: false });
  const [done, setDone] = useState([false, false, true]);
  const [toggle, setToggle] = useState(true);
  const [nav, setNav] = useState("meetings");

  // rem is root-relative, so the text size goes on the document root.
  useEffect(() => {
    document.documentElement.style.fontSize = `${size}%`;
    return () => {
      document.documentElement.style.fontSize = "";
    };
  }, [size]);

  const navItems = (
    [
      ["today", "today", "Today", "Ctrl+1"],
      ["meetings", "meetings", "Meetings", "Ctrl+2"],
      ["library", "library", "Library", "Ctrl+3"],
      ["topics", "topics", "Topics", "Ctrl+4"],
      ["automations", "automations", "Automations", "Ctrl+5"],
    ] as const
  ).map(([id, icon, label, key]) => (
    <WSidebarItem key={id} icon={icon} label={label} shortcut={key} selected={nav === id} onPress={() => setNav(id)} />
  ));

  const cls = `w-thread g-root${mode === "system" ? "" : ` w-mode-${mode}`}`;

  return (
    <div className={cls}>
      <WRoot>
        <header className="g-head">
          <h1 className="g-h1">{"Whimbrel design system"}</h1>
          <div className="g-switch" role="group" aria-label="Mode">
            {MODES.map(([id, label]) => (
              <button key={id} type="button" className="g-opt" aria-pressed={mode === id} onClick={() => setMode(id)}>
                {label}
              </button>
            ))}
          </div>
          <div className="g-switch" role="group" aria-label="Text size">
            {SIZES.map((s) => (
              <button key={s} type="button" className="g-opt" aria-pressed={size === s} onClick={() => setSize(s)}>
                {`${s}%`}
              </button>
            ))}
          </div>
        </header>

        <main className="g-main" hidden={init.only === "settings"}>
          <Section title="Finished note, with sources in the margin">
            <div className="g-note">
              <div className="g-note-page">
                <p className="g-eyebrow">{NOTE.meta}</p>
                <h3 className="g-note-title">{NOTE.title}</h3>
                <p className="g-people">{NOTE.people}</p>
                <p className="g-eyebrow g-gap">{"Your notes"}</p>
                {NOTE.yours.map((line) => (
                  <WNoteBlock key={line} kind="paragraph" author="you" text={line} />
                ))}
                <WNoteBlock kind="heading" author="whimbrel" text="Renewal terms" />
                {NOTE.renewal.map((b) => (
                  <WNoteBlock key={b.text} kind="paragraph" author="whimbrel" text={b.text} sources={b.sources} selected={picked !== null && b.sources.includes(picked)} selectedSource={picked} onSelectSource={setPicked} />
                ))}
                <WNoteBlock kind="heading" author="whimbrel" text="Risks" />
                <WNoteBlock kind="paragraph" author="whimbrel" text={NOTE.risk.text} sources={NOTE.risk.sources} selected={picked === 3} selectedSource={picked} onSelectSource={setPicked} />
                <WNoteBlock kind="heading" author="whimbrel" text="Next steps" />
                <ul>
                  {NOTE.steps.map((s, i) => (
                    <WActionItemRow key={s.text} text={s.text} owner={s.owner} due={s.due} sourceTime={s.at} sources={s.sources} selectedSource={picked} onSelectSource={setPicked} done={done[i]} onToggle={(v) => setDone(done.map((d, j) => (j === i ? v : d)))} onOpenSource={noop} />
                  ))}
                </ul>
              </div>
              <aside className="g-margin" aria-label="Sources">
                <p className="g-margin-note">{"Whimbrel wrote the blue lines. Each number points to the moment it came from."}</p>
                {SOURCES.map((s) => (
                  <WSourceCard key={s.number} number={s.number} appName={s.who} appIcon="mic" kind="Transcript moment" time={s.time} quote={s.quote} selected={picked === s.number} openLabel={`Open in transcript at ${s.time}`} onOpen={noop} />
                ))}
              </aside>
            </div>
          </Section>

          <Section title="Recording indicator">
            <Case label="Bar: recording" wide>
              <WRecordingIndicator
                state="recording"
                size="bar"
                elapsedSeconds={848}
                actions={
                  <>
                    <WButton icon="pause">{"Pause"}</WButton>
                    <WButton icon="stop">{"Stop"}</WButton>
                  </>
                }
              />
            </Case>
            <Case label="Bar: recording with level meter" wide>
              <WRecordingIndicator
                state="recording"
                size="bar"
                elapsedSeconds={848}
                meter={<WLevelMeter level={0.5} arriving />}
                actions={
                  <>
                    <WButton icon="pause">{"Pause"}</WButton>
                    <WButton icon="stop">{"Stop"}</WButton>
                  </>
                }
              />
            </Case>
            <Case label="Bar: paused" wide>
              <WRecordingIndicator state="paused" size="bar" elapsedSeconds={848} actions={<WButton variant="primary">{"Resume"}</WButton>} />
            </Case>
            <Case label="Bar: paused with a flat meter" wide>
              <WRecordingIndicator state="paused" size="bar" elapsedSeconds={848} meter={<WLevelMeter level={0} arriving={false} idle />} actions={<WButton variant="primary">{"Resume"}</WButton>} />
            </Case>
            <Case label="Bar: starting" wide>
              <WRecordingIndicator state="starting" size="bar" />
            </Case>
            <Case label="Bar: resuming" wide>
              <WRecordingIndicator state="resuming" size="bar" meter={<WLevelMeter level={0} arriving={false} />} actions={<WButton disabled>{"Resume"}</WButton>} />
            </Case>
            <Case label="Bar: processing" wide>
              <WRecordingIndicator state="processing" size="bar" />
            </Case>
            <Case label="Bar: off" wide>
              <WRecordingIndicator state="off" size="bar" />
            </Case>
            <Case label="Bar: problem" wide>
              <WRecordingIndicator state="problem" size="bar" reason="No microphone found" actions={<WButton>{"Try again"}</WButton>} />
            </Case>
            <Case label="Bar: problem with the time" wide>
              <WRecordingIndicator state="problem" size="bar" elapsedSeconds={848} reason="Some audio may be missing" actions={<WButton variant="primary">{"Stop"}</WButton>} />
            </Case>
            <Case label="Level meter: arriving, and not">
              <WLevelMeter level={0.6} arriving />
              <WLevelMeter level={0} arriving={false} />
            </Case>
            {(["recording", "paused", "processing", "off", "problem"] as const).map((s) => (
              <Case key={s} label={`Pill: ${s}`}>
                <WRecordingIndicator state={s} size="pill" elapsedSeconds={848} reason="No microphone" />
              </Case>
            ))}
            {(["recording", "paused", "processing", "off", "problem"] as const).map((s) => (
              <Case key={s} label={`Menu: ${s}`}>
                <WRecordingIndicator state={s} size="menu" elapsedSeconds={848} reason="No microphone" />
              </Case>
            ))}
          </Section>

          <Section title="Capture state, consent banner">
            <Case label="Memory on, menu open" wide>
              <div className="g-menu-room">
                <WCaptureState state="on" detail="Saving context from your screen since 8:02 AM" choices={PAUSE_CHOICES} onChoose={noop} onWhatIsCaptured={noop} open onOpenChange={noop} />
              </div>
            </Case>
            <Case label="Paused until 10:52 AM">
              <WCaptureState state="paused" pausedUntil="10:52 AM" choices={PAUSE_CHOICES} onChoose={noop} onWhatIsCaptured={noop} open={false} onOpenChange={noop} />
            </Case>
            <Case label="Memory off">
              <WCaptureState state="off" choices={PAUSE_CHOICES} onChoose={noop} onWhatIsCaptured={noop} open={false} onOpenChange={noop} />
            </Case>
            <Case label="Consent banner" wide>
              <WConsentBanner onCopy={noop} onDone={noop} />
            </Case>
            <Case label="Consent banner, after Copy notice" wide>
              <WConsentBanner onCopy={noop} onDone={noop} copied />
            </Case>
          </Section>

          <Section title="Small windows: frame, recording row, memory control">
            <Case label="Meeting detected, with the countdown">
              <div className="g-window">
                <WWindowFrame
                  title="Meeting detected"
                  lines={[<span className="w-win-strong">{"Northwind security review"}</span>, "Record it? Audio stays on this PC."]}
                  countdown={<span>{"Recording starts in 8 s"}</span>}
                  actions={
                    <>
                      <WButton variant="primary">{"Record"}</WButton>
                      <WButton>{"Record & open"}</WButton>
                      <WButton title="Stops the countdown. Whimbrel will not ask about this meeting again.">{"Don\u2019t record"}</WButton>
                    </>
                  }
                />
              </div>
            </Case>
            <Case label="Still in a meeting?">
              <div className="g-window">
                <WWindowFrame
                  title="Still in a meeting?"
                  lines={["No one has spoken for 15 minutes."]}
                  actions={
                    <>
                      <WButton variant="primary">{"Keep recording"}</WButton>
                      <WButton>{"Stop"}</WButton>
                    </>
                  }
                />
              </div>
            </Case>
            <Case label="Still recording? (hourly, with its countdown)">
              <div className="g-window">
                <WWindowFrame
                  title="Still recording?"
                  lines={["This recording has been running for a while."]}
                  countdown={<span>{"Stopping in 4:59"}</span>}
                  actions={
                    <>
                      <WButton variant="primary">{"Keep recording"}</WButton>
                      <WButton>{"Stop now"}</WButton>
                    </>
                  }
                />
              </div>
            </Case>
            <Case label="Frame with a long body: the lines scroll under a cue, the actions stay">
              <div className="g-window g-window-short">
                <WWindowFrame
                  title="Your disk is almost full"
                  lines={["Only 4.5 GB left on C:. Whimbrel may stop recording.", "Only 8.1 GB left on D:, where the recordings are kept.", "Free up space in Settings, under Storage, or move the recordings to another drive."]}
                  actions={
                    <>
                      <WButton variant="primary">{"Free up space…"}</WButton>
                      <WButton>{"Dismiss"}</WButton>
                    </>
                  }
                />
              </div>
            </Case>
            <Case label="Recording row: recording, paused, problem, and a problem while paused">
              <div className="g-window g-stack">
                <WRecordingRow state="recording" elapsedSeconds={848} actions={<><WButton icon="pause">{"Pause"}</WButton><WButton icon="stop" variant="primary">{"Stop"}</WButton></>} />
                <WRecordingRow state="paused" elapsedSeconds={848} actions={<><WButton icon="play">{"Resume"}</WButton><WButton icon="stop" variant="primary">{"Stop"}</WButton></>} />
                <WRecordingRow state="problem" elapsedSeconds={848} reason="The microphone isn't delivering audio, so this meeting records system audio only." actions={<WButton icon="stop" variant="primary">{"Stop"}</WButton>} />
                <WRecordingRow state="paused" word="Recording paused" problem elapsedSeconds={728} reason="Your headset switched to call quality, so the meeting's audio may sound muffled." actions={<><WButton icon="play">{"Resume"}</WButton><WButton icon="stop" variant="primary">{"Stop"}</WButton></>} />
              </div>
            </Case>
            <Case label="Memory control: on, paused, paused by the app, off, stopped">
              <div className="g-window g-stack">
                {(["on", "paused", "pausedByApp", "off", "stopped"] as const).map((state) => (
                  <WMemoryControl
                    key={state}
                    state={state}
                    pausedUntil="3:10 PM"
                    notes={state === "stopped" ? ["Whimbrel could not start memory."] : state === "on" ? ["Synced 10:20 AM"] : undefined}
                    onOpenSettings={noop}
                    choices={[{ id: "15m", label: "15 minutes" }, { id: "1h", label: "1 hour" }, { id: "tomorrow", label: "Until tomorrow" }]}
                    onPause={noop}
                    onResumeNow={noop}
                    onTurnOff={noop}
                    onTurnOn={noop}
                    onWhatIsCaptured={noop}
                  />
                ))}
              </div>
            </Case>
          </Section>

          <Section title="Sidebar, search, filters">
            <Case label="Sidebar with capture state in the footer">
              <div className="g-sidebar-frame">
                <WSidebar
                  items={
                    <>
                      {navItems}
                      <WSidebarItem icon="spark" label="Ask" badge={3} onPress={noop} />
                    </>
                  }
                  footer={<WCaptureState state="on" choices={PAUSE_CHOICES} onChoose={noop} onWhatIsCaptured={noop} open={false} onOpenChange={noop} />}
                />
              </div>
            </Case>
            <Case label="Search and filter chips">
              <div className="g-stack">
                <WSearchBar value={query} onChange={setQuery} />
                <WSearchBar value="Northwind" onChange={noop} />
                <div className="g-row">
                  <WFilterChip label="People" icon="people" menu onPress={noop} />
                  <WFilterChip label="Apps" icon="apps" menu onPress={noop} />
                  <WFilterChip label="Dates" icon="calendar" menu onPress={noop} />
                </div>
                <div className="g-row">
                  <WFilterChip label="Priya Shah" selected={chips.people} onPress={() => setChips({ ...chips, people: !chips.people })} onRemove={noop} />
                  <WFilterChip label="Excel" selected={chips.apps} onPress={() => setChips({ ...chips, apps: !chips.apps })} />
                  <WFilterChip label="Last 7 days" selected onPress={noop} onRemove={noop} />
                </div>
              </div>
            </Case>
          </Section>

          <Section title="Thread: spine, meetings, memory">
            <Case label="Spine" wide>
              <WSpine label="Today">
                <WSpineStop time="3:00 PM" node="upcoming">
                  <WMeetingCard variant="upcoming" title="Hiring debrief" attendees={[PEOPLE.priya]} soon="in 5 hr 8 min" primaryLabel="Open" onPrimary={noop} />
                </WSpineStop>
                <WSpineStop time="10:00 AM" node="upcoming">
                  <WMeetingCard variant="next" title="Pricing review" when="Today, 10:00 AM. Last met Tue, Nov 10." soon="in 8 minutes" attendees={[PEOPLE.sam, PEOPLE.priya, "You"]} primaryLabel="Start notes" primaryHint="Ctrl+Enter" onPrimary={noop} secondaryLabel="Open notes from Nov 10" onSecondary={noop} />
                </WSpineStop>
                <WSpineStop time="9:52 AM" node="now" connector="solid">
                  <WNowMarker text="Memory on since 8:02 AM" />
                </WSpineStop>
                <WSpineStop time="22 min" node="past" connector="solid">
                  <WMemoryBlock range="9:30 to 9:52 AM" items={MEMORY_ITEMS} />
                </WSpineStop>
                <WSpineStop time="10:14 AM" timeNote="Now" node="recording" connector="solid">
                  <WMeetingCard variant="past" title="Northwind renewal call" attendees={[PEOPLE.dana, PEOPLE.jo]} gist="Two-year renewal if the price holds. Security questionnaire still open." primaryLabel="Open notes" onPrimary={noop} />
                </WSpineStop>
                <WSpineStop time="Mon" node="past" last>
                  <WMeetingCard variant="past" title="Roadmap check-in" attendees={[PEOPLE.sam]} primaryLabel="Open notes" onPrimary={noop} />
                </WSpineStop>
              </WSpine>
            </Case>
            <Case label="Speaker chips">
              <div className="g-row">
                <WSpeakerChip name={PEOPLE.sam} />
                <WSpeakerChip name={PEOPLE.jo} />
                <WSpeakerChip name={PEOPLE.alex} size="small" />
              </div>
            </Case>
          </Section>

          <Section title="Home choices: next meeting, labels, ask bar, ruled line, link actions">
            <div style={{ display: "grid", gap: 16 }}>
              {(["card", "scoreboard", "infobox"] as const).map((variant) => (
                <WNextMeeting key={variant} variant={variant} title="Pricing review" time="10:00 AM" when="Today, 10:00 AM" soon="in 8 min" attendees={[PEOPLE.sam, PEOPLE.priya]} primaryLabel="Start" onPrimary={noop} secondaryLabel="Open brief" onSecondary={noop} />
              ))}
              <WNextMeeting variant="card" links title="Pricing review" time="10:00 AM" soon="in 8 min" attendees={[PEOPLE.sam]} primaryLabel="Start" onPrimary={noop} secondaryLabel="Open brief" onSecondary={noop} />
              {(["plain", "bracketed", "smallCaps"] as const).map((v) => (
                <WSectionLabel key={v} variant={v}>
                  Open threads
                </WSectionLabel>
              ))}
              {(["field", "prompt"] as const).map((v) => (
                <WAskBar key={v} variant={v} value="" onChange={noop} onSubmit={noop} placeholder="Ask Whimbrel about your meetings" label="Ask Whimbrel" submitLabel="Ask" hint="Ctrl+K" />
              ))}
              <WSpine label="Ruled time line" variant="ruled">
                <WSpineStop time="10:31 AM" node="upcoming">
                  <WMeetingCard variant="upcoming" title="Northwind security review" attendees={[PEOPLE.sam]} soon="in 9 min" primaryLabel="Start" onPrimary={noop} />
                </WSpineStop>
                <WSpineNow time="10:22 AM" text="Memory on" />
              </WSpine>
            </div>
          </Section>
          <Section title="Transcript, note blocks, sources">
            <Case label="Transcript: normal, cited, live">
              <WTranscriptView label="Transcript">
                <WTranscriptLine {...TRANSCRIPT[0]} />
                <WTranscriptLine {...TRANSCRIPT[1]} state="cited" />
                <WTranscriptLine {...TRANSCRIPT[2]} />
                <WTranscriptLine {...TRANSCRIPT[3]} state="live" />
              </WTranscriptView>
            </Case>
            <Case label="Note blocks: you, Whimbrel, selected">
              <div className="g-stack">
                <WNoteBlock kind="heading" author="you" text="Pricing" />
                <WNoteBlock kind="paragraph" author="you" text="annual discount 15 or 20?" />
                <WNoteBlock kind="bullet" author="you" text="priya: check churn on starter first" />
                <WNoteBlock kind="check" author="you" text="send deck to Sam" checked={false} onCheck={noop} />
                <WNoteBlock kind="bullet" author="whimbrel" text="Start the annual discount at 15 percent." sources={[1]} selectedSource={null} onSelectSource={noop} />
                <WNoteBlock kind="paragraph" author="whimbrel" text="Priya will check churn on Starter first." sources={[2, 3]} selected selectedSource={3} onSelectSource={noop} />
              </div>
            </Case>
            <Case label="Source marks: rest, selected">
              <div className="g-row">
                <WSourceMark number={1} onSelect={noop} />
                <WSourceMark number={2} selected onSelect={noop} />
              </div>
            </Case>
            <Case label="Source card, card size, and selected">
              <div className="g-stack">
                <WSourceCard size="card" number={1} appName="Excel" appIcon="table" kind="Seen on screen" title="Pricing v3.xlsx" time="09:41" quote="Starter, annual: 15 percent" onOpen={noop} />
                <WSourceCard size="card" number={2} appName="Priya Shah" appIcon="mic" kind="Transcript moment" time="10:12" quote="So a usage-based tier, on top of seats?" selected onOpen={noop} />
              </div>
            </Case>
            <Case label="Action items: open, with source, done" wide>
              <ul>
                <WActionItemRow text="Send the two-year renewal quote to Jo." owner={PEOPLE.sam} due="Due Thu, Nov 19" sourceTime="28:50" done={false} onToggle={noop} onOpenSource={noop} />
                <WActionItemRow text="Check that 55 seats still fits the current price." owner={PEOPLE.alex} due="Due Tue, Nov 24" done={false} onToggle={noop} />
                <WActionItemRow text="Answer the open items on the questionnaire." owner={PEOPLE.dana} due="Due Fri, Nov 20" sourceTime="29:31" done onToggle={noop} onOpenSource={noop} />
              </ul>
            </Case>
          </Section>

          <Section title="Settings rows, buttons">
            <Case label="Settings rows" wide>
              <div>
                <WSettingsRow variant="toggle" label="Work memory" description="Save context from your screen while you work." checked={toggle} onToggle={setToggle} />
                <WSettingsRow variant="toggle" label="Meeting notes" checked={false} onToggle={noop} />
                <WSettingsRow variant="value" label="Text size" description="Follows Windows by default." value="Windows setting" onPress={noop} />
                <WSettingsRow variant="button" label="Export notes" actionLabel="Export" onPress={noop} />
                <WSettingsRow variant="destructive" label="Delete everything" description="Removes all notes and memory from this PC." actionLabel="Delete" onPress={noop} />
              </div>
            </Case>
            <Case label="Buttons: primary, secondary, quiet, destructive, hint, disabled" wide>
              <div className="g-row">
                <WButton variant="primary" icon="check">{"Save"}</WButton>
                <WButton>{"Cancel"}</WButton>
                <WButton variant="quiet" icon="copy">{"Copy notice"}</WButton>
                <WButton variant="destructive">{"Delete"}</WButton>
                <WButton variant="primary" icon="document" hint="Ctrl+Enter">{"Start notes"}</WButton>
                <WButton disabled>{"Disabled"}</WButton>
              </div>
            </Case>
          </Section>

          <Section title="Flow parts: launch and first run">
            <Case label="Step line, title, paragraph, fields and actions" wide>
              <div className="g-flow">
                <p className="w-flow-step">{"Step 1 of 4"}</p>
                <h2 className="w-flow-title">{"Create your recovery key"}</h2>
                <p>{"Whimbrel will encrypt the memory database on this PC with a key kept in Windows Credential Manager."}</p>
                <WPanel>
                  <WTextField label="Passphrase" type="password" value="a long passphrase" onChange={noop} help="At least 12 characters. A short sentence you will remember works well." status={{ text: "17 of at least 12 characters", tone: "ok" }} />
                  <WTextField label="Type it again" type="password" value="a long" onChange={noop} error="The two passphrases do not match." />
                  <WTextField label="Passphrase" type="password" value="" onChange={noop} disabled />
                </WPanel>
                <div className="w-flow-actions">
                  <WButton>{"Quit Whimbrel"}</WButton>
                  <WButton variant="primary">{"Create recovery key"}</WButton>
                </div>
              </div>
            </Case>
            <Case label="Checkbox: on, off, disabled">
              <WPanel>
                <WCheckbox label="I wrote down all 24 words, in order" checked onChange={noop} />
                <WCheckbox label="I have kept my recovery file somewhere else" checked={false} onChange={noop} />
                <WCheckbox label="Waiting for the test" checked={false} onChange={noop} disabled />
              </WPanel>
            </Case>
            <Case label="Notices: info, saved, warning, error">
              <div className="g-stack">
                <WNotice>{"Your previous data was deleted. This is a new, empty library."}</WNotice>
                <WNotice tone="ok">{"Saved to E:\\Whimbrel recovery key.txt"}</WNotice>
                <WNotice tone="warning" strong>{"These words are the key itself. Your passphrase does not protect them."}</WNotice>
                <WNotice tone="error">{"The file could not be saved there. Try another folder."}</WNotice>
              </div>
            </Case>
            <Case label="Progress line">
              <div className="g-stack">
                <WProgressLine label="Encrypting the memory database" value={0.42} />
                <WProgressLine label="Encrypting the memory database" value={1} />
              </div>
            </Case>
            <Case label="Numbered list, three columns of eight">
              <WNumberedList label="Words" items={["orbit", "lantern", "meadow", "copper", "harbor", "velvet", "anchor", "pebble", "summit", "marble", "thistle", "ember", "quartz", "willow", "saddle", "nimbus", "tundra", "fabric", "glacier", "hollow", "ivory", "juniper", "kettle", "lagoon"]} protectedFromCopy />
            </Case>
            <Case label="Segmented choice">
              <WSegmented label="Recovery key type" value="file" onChange={noop} options={[{ value: "file", label: "Recovery file" }, { value: "words", label: "24 words" }]} />
            </Case>
            <Case label="More options (closed)">
              <div className="w-flow-actions">
                <WMoreMenu items={[{ key: "a", label: "Put back the unencrypted copy", onPress: noop }]} />
                <WButton>{"Quit Whimbrel"}</WButton>
                <WButton variant="primary">{"Try again"}</WButton>
              </div>
            </Case>
            <Case label="Question: a dialog over the page" wide>
              <div className="g-dialog-host">
                <WDialog title="Reset encryption status and start empty?" body="Nothing is deleted: only Whimbrel's record of the earlier encryption is set aside." onCancel={noop}>
                  <WButton>{"Cancel"}</WButton>
                  <WButton variant="destructive">{"Reset and start empty"}</WButton>
                </WDialog>
              </div>
            </Case>
          </Section>

          <Section title="Empty, loading, error">
            <Case label="Empty">
              <WEmptyState icon="meetings" title="No meetings yet" help="Your next meeting shows up here." actionLabel="Open calendar" onAction={noop} />
            </Case>
            <Case label="Loading">
              <WLoadingState title="Loading notes" help="This takes a moment the first time." />
            </Case>
            <Case label="Error">
              <WErrorState title="Could not load notes" help="Whimbrel could not read your notes. Your notes are safe on this PC." onAction={noop} />
            </Case>
          </Section>

          <Section title="Icons">
            <div className="g-icons">
              {W_ICON_NAMES.map((n) => (
                <div key={n} className="g-icon">
                  <WIcon name={n} />
                  <span>{n}</span>
                </div>
              ))}
            </div>
          </Section>
        </main>
        <SettingsGallery />
      </WRoot>
    </div>
  );
}
