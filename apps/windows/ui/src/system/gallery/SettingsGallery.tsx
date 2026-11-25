import { useState, type ReactNode } from "react";
import {
  WButton,
  WConfirmDialog,
  WIconButton,
  WInlineConfirm,
  WMoreMenu,
  WNoticeBox,
  WSettingsCard,
  WSettingsChoiceRow,
  WSettingsControlRow,
  WSettingsDisclosure,
  WSettingsLayout,
  WSettingsLinkRow,
  WSettingsList,
  WSettingsListRow,
  WSettingsNav,
  WSettingsNote,
  WSettingsNumberRow,
  WSettingsProgressRow,
  WSettingsRow,
  WSettingsSearchResults,
  WSettingsSecretRow,
  WSettingsSelectRow,
  WSettingsShortcutRow,
  WSettingsStatusRow,
  WSettingsTextRow,
  WSettingsToggleRow,
  WSettingsValueList,
  WSkinCard,
  WSkinGrid,
  WStatusChip,
  type WSettingsNavItem,
} from "..";

// Every Settings part in every state, for the screenshots and for looking at by hand. Each group has a data-shot name,
// and e2e/gallery.shots.ts photographs each group on its own. Labels here are plain strings, not t(): this screen is
// not shipped. Open it at /gallery.html?only=settings.

const noop = () => {};

function Group({ shot, title, children }: { shot: string; title: string; children: ReactNode }) {
  return (
    <section className="g-section" data-shot={shot}>
      <h2 className="g-h2">{title}</h2>
      <div className="g-body">{children}</div>
    </section>
  );
}
function Case({ label, children, wide }: { label: string; children: ReactNode; wide?: boolean }) {
  return (
    <div className={wide ? "g-case g-wide" : "g-case"} data-case>
      <div className="g-label">{label}</div>
      {children}
    </div>
  );
}

const SECTIONS: WSettingsNavItem[] = [
  { id: "general", label: "General", icon: "settings" },
  { id: "capture", label: "Capture and privacy", icon: "record" },
  { id: "meetings", label: "Meetings and recordings", icon: "meetings" },
  { id: "models", label: "Models and AI providers", icon: "spark" },
  { id: "profile", label: "Profile", icon: "people", badge: 2, badgeLabel: "2 actions waiting for approval" },
  { id: "storage", label: "Storage and keeping data", icon: "library" },
];

const MODELS_PATH = "C:\\Users\\example.person\\AppData\\Local\\Whimbrel\\models\\transcription\\ggml-large-v3-turbo-q5_0-extra-long-file-name.bin";

function NavDemo({ compact }: { compact?: boolean }) {
  const [section, setSection] = useState("capture");
  return (
    <WSettingsLayout nav={<WSettingsNav label="Settings sections" items={SECTIONS} selected={section} onSelect={setSection} compact={compact} />}>
      <WSettingsCard headingLevel={2} title={SECTIONS.find((s) => s.id === section)?.label ?? ""} description="The content column keeps its width when the list becomes a select.">
        <WSettingsToggleRow label="Capture the text of the window you are working in" checked onToggle={noop} />
        <WSettingsToggleRow label="Show screen captures in the Library" checked={false} onToggle={noop} />
      </WSettingsCard>
    </WSettingsLayout>
  );
}

export function SettingsGallery() {
  const [toggle, setToggle] = useState(true);
  const [check, setCheck] = useState(false);
  const [lang, setLang] = useState("auto");
  const [mode, setMode] = useState<"auto" | "light" | "dark">("auto");
  const [days, setDays] = useState(30);
  const [forever, setForever] = useState(false);
  const [lead, setLead] = useState(5);
  const [text, setText] = useState("Please keep the notes short.");
  const [saved, setSaved] = useState(false);
  const [recording, setRecording] = useState(false);
  const [open, setOpen] = useState(true);
  const [apps, setApps] = useState(["Example Notes", "Sample Chat", "Test Browser"]);
  const [editing, setEditing] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>("Sample Chat");
  const [filter, setFilter] = useState("");
  const [skin, setSkin] = useState("whimbrel");
  const [palette, setPalette] = useState("dusk");
  const [dialog, setDialog] = useState<"plain" | "typed">("typed");
  const [ack, setAck] = useState(false);
  const shown = apps.filter((a) => a.toLowerCase().includes(filter.toLowerCase()));
  const palettes = [
    { id: "dusk", label: "Dusk", color: "var(--color-accent-primary)" },
    { id: "meadow", label: "Meadow", color: "var(--color-success)" },
    { id: "ember", label: "Ember", color: "var(--color-warning)" },
  ];
  const prev = <span className="g-skin-prev" />;

  return (
    <main className="g-main">
      <Group shot="nav" title="Settings parts: section list, search results">
        <Case label="Layout wide: section list, one Tab stop, Up and Down move" wide>
          <div className="g-frame g-frame-wide">
            <NavDemo />
          </div>
        </Case>
        <Case label="Layout narrow (under 40rem): the list is a select above the content" wide>
          <div className="g-frame g-frame-narrow">
            <NavDemo />
          </div>
        </Case>
        <Case label="Search results, and none">
          <div className="g-stack">
            <WSettingsSearchResults
              summary="2 settings found"
              results={[
                { id: "a", title: "Keep screen text for", section: "Capture and privacy" },
                { id: "b", title: "Keep meetings and recordings for", section: "Storage and keeping data" },
              ]}
              onOpen={noop}
            />
            <WSettingsSearchResults results={[]} onOpen={noop} />
          </div>
        </Case>
        <Case label="Status chips: neutral, good, warning, problem">
          <div className="g-row">
            <WStatusChip>{"Off"}</WStatusChip>
            <WStatusChip tone="good">{"Encrypted"}</WStatusChip>
            <WStatusChip tone="warning">{"Paused until 10:52"}</WStatusChip>
            <WStatusChip tone="problem">{"Not running: no microphone found"}</WStatusChip>
          </div>
        </Case>
      </Group>

      <Group shot="rows" title="Settings parts: card and rows">
        <Case label="Card: heading, description, status chip, rows (switch, checkbox, select, choice, status, button)" wide>
          <WSettingsCard
            id="card-general"
            headingLevel={2}
            title="General"
            description="How Whimbrel starts and how it looks. Each PC keeps its own choices."
            status={{ tone: "good", children: "Up to date" }}
          >
            <WSettingsToggleRow label="Start Whimbrel when you sign in to Windows" description="Whimbrel opens in the tray, not in front of your work." checked={toggle} onToggle={setToggle} />
            <WSettingsToggleRow kind="checkbox" label="Show screen captures in the Library" description="Off by default. Captured text is still searchable; this only shows the images they came from." checked={check} onToggle={setCheck} />
            <WSettingsSelectRow label="Language" description="The words in menus and buttons." value={lang} onChange={setLang} options={[{ value: "auto", label: "Automatic" }, { value: "de", label: "Deutsch" }, { value: "es", label: "Español" }, { value: "fr", label: "Français" }]} />
            <WSettingsChoiceRow label="Appearance" value={mode} onChange={setMode} options={[{ value: "auto", label: "Automatic" }, { value: "light", label: "Light" }, { value: "dark", label: "Dark" }]} />
            <WSettingsStatusRow label="Updates" description="Checked once a day." status={{ tone: "good", children: "Up to date" }} action={<WButton>{"Check now"}</WButton>} />
            <WSettingsControlRow label="Export or reset" description="Several buttons in one row.">
              <div className="g-row">
                <WButton>{"Export"}</WButton>
                <WButton>{"Import"}</WButton>
                <WButton variant="destructive">{"Reset"}</WButton>
              </div>
            </WSettingsControlRow>
            <WSettingsRow variant="value" label="Models" description="Choose how Whimbrel writes transcripts." value="Local" onPress={noop} />
            <WSettingsRow variant="destructive" label="Delete everything" description="Removes your meetings, memory, recordings and logs from this PC." actionLabel="Delete" onPress={noop} />
            <WSettingsNote>{"Available in release builds only."}</WSettingsNote>
          </WSettingsCard>
        </Case>
        <Case label="Disabled, with the reason shown, and a row you were sent to from search" wide>
          <WSettingsCard headingLevel={3} title="Capture and privacy" loading={false}>
            <WSettingsToggleRow label="Start Whimbrel when you sign in to Windows" description="Whimbrel opens in the tray." checked={false} onToggle={noop} disabled disabledReason="Available in release builds only." />
            <WSettingsToggleRow kind="checkbox" label="Keep a copy of every recording" description="Takes more space on this PC." checked onToggle={noop} disabled disabledReason="A policy on this PC sets this." />
            <WSettingsSelectRow label="Quality" value="high" onChange={noop} disabled disabledReason="Choose a model first." options={[{ value: "high", label: "High" }]} />
            <WSettingsChoiceRow label="Appearance" value="dark" onChange={noop} disabled disabledReason="The high contrast theme is on, so the colors are set by Windows." options={[{ value: "light", label: "Light" }, { value: "dark", label: "Dark" }]} />
            <WSettingsToggleRow label="Memory on" description="You jumped here from search." checked onToggle={noop} highlighted />
            <WSettingsRow variant="button" label="Open the log folder" actionLabel="Open" onPress={noop} disabled disabledReason="The folder does not exist yet." />
          </WSettingsCard>
        </Case>
        <Case label="Card: loading, and with an error line">
          <div className="g-stack">
            <WSettingsCard headingLevel={2} title="Storage" loading />
            <WSettingsCard headingLevel={2} title="Sync" error="Could not reach the sync service. Your data is safe on this PC.">
              <WSettingsToggleRow label="Sync this PC" checked={false} onToggle={noop} />
            </WSettingsCard>
          </div>
        </Case>
      </Group>

      <Group shot="fields" title="Settings parts: number, text, secret">
        <Case label="Number rows: unit, companion checkbox keeps the number, invalid" wide>
          <WSettingsCard headingLevel={3} title="Keeping data">
            <WSettingsNumberRow label="Screen text" description="Older screen text is deleted each night." value={days} min={1} max={3650} unit="days" onCommit={setDays} companion={{ label: "Keep forever", checked: forever, onChange: setForever }} />
            <WSettingsNumberRow label="Meetings and recordings" value={365} min={1} max={3650} unit="days" onCommit={noop} companion={{ label: "Keep forever", checked: true, onChange: noop }} />
            <WSettingsNumberRow label="Prepare notes before a meeting" value={lead} min={5} max={120} unit="minutes" onCommit={setLead} />
            <WSettingsNumberRow label="Chats" value={30} min={1} max={3650} unit="days" onCommit={noop} disabled disabledReason="Sign in to change how long chats are kept." />
            <WSettingsNumberRow label="Alert volume" value={3} min={1} max={10} onCommit={noop} error="Enter a number from 1 to 10." />
          </WSettingsCard>
        </Case>
        <Case label="Text row (saves when you leave it), text area, required" wide>
          <WSettingsCard headingLevel={3} title="Notes">
            <WSettingsTextRow label="Your name in notes" value="Sam Example" onCommit={noop} required />
            <WSettingsTextRow label="Extra instructions" description="Added to every summary." value={text} onCommit={setText} rows={3} />
            <WSettingsTextRow label="Server path" value={MODELS_PATH} onCommit={noop} mono rows={3} />
          </WSettingsCard>
        </Case>
        <Case label="Secret row: empty, editing a saved key, saved (nothing but a mask in the page), error" wide>
          <WSettingsCard headingLevel={3} title="AI provider keys">
            <WSettingsSecretRow label="Example provider key" description="Whimbrel keeps it in Windows Credential Manager." hint="Keys start with sk-example-not-a-key." saved={saved} onSave={() => { setSaved(true); return true; }} onRemove={() => setSaved(false)} removeQuestion="Remove this key?" status={saved ? "Checked a moment ago: the key works." : undefined} />
            <WSettingsSecretRow label="Second provider key" saved={false} onSave={() => false} error="That key was refused. Check it and try again." />
            <WSettingsSecretRow label="Saving" saved={false} onSave={() => false} busy status="Checking the key…" />
          </WSettingsCard>
        </Case>
      </Group>

      <Group shot="lists" title="Settings parts: lists, links, values">
        <Case label="List: filter, rows (caption, chip, actions, switch), inline edit, inline confirm, add, empty" wide>
          <WSettingsCard headingLevel={3} title="Excluded apps">
            <WSettingsList
              label="Excluded apps"
              description="Whimbrel never captures these."
              filter={{ label: "Filter excluded apps", value: filter, onChange: setFilter, placeholder: "Filter" }}
              isEmpty={shown.length === 0}
              empty="No apps excluded."
              add={{ label: "App to exclude", placeholder: "Example.exe", existing: apps, onAdd: (v) => void setApps((a) => [...a, v]) }}
            >
              {shown.map((a, i) =>
                editing === a ? (
                  <WSettingsListRow key={a} title={a} edit={{ label: "Name", value: a, onSave: (v) => (setApps((l) => l.map((x) => (x === a ? v : x))), setEditing(null)), onCancel: () => setEditing(null) }} />
                ) : (
                  <WSettingsListRow
                    key={a}
                    title={a}
                    caption={i === 0 ? "C:\\Program Files\\Example Notes\\notes.exe" : "Seen 2 minutes ago"}
                    chip={i === 1 ? { tone: "warning", children: "Not installed" } : undefined}
                    toggle={i === 2 ? { checked: true, onChange: noop } : undefined}
                    actions={
                      removing === a ? (
                        <WInlineConfirm question={`Stop excluding ${a}?`} confirmLabel="Remove" onConfirm={() => (setApps((l) => l.filter((x) => x !== a)), setRemoving(null))} onCancel={() => setRemoving(null)} returnFocusTo={`rm-${i}`} />
                      ) : (
                        <>
                          <WButton variant="quiet" onClick={() => setEditing(a)}>{"Rename"}</WButton>
                          <WButton variant="quiet" id={`rm-${i}`} onClick={() => setRemoving(a)}>{"Remove"}</WButton>
                        </>
                      )
                    }
                  />
                ),
              )}
            </WSettingsList>
            <WSettingsList label="Remembered voices" isEmpty empty="No voices remembered yet." />
          </WSettingsCard>
        </Case>
        <Case label="Links: in the app, to the web" wide>
          <WSettingsCard headingLevel={3} title="Help">
            <WSettingsLinkRow kind="section" label="Models and AI providers" description="Pick where transcripts and summaries come from." linkText="Open" onPress={noop} />
            <WSettingsLinkRow kind="external" label="Privacy policy" description="What Whimbrel does with your data." href="https://example.com/privacy" linkText="Read the privacy policy" onPress={noop} />
          </WSettingsCard>
        </Case>
        <Case label="Values: a long path wraps, copy buttons, no sideways scroll" wide>
          <WSettingsCard headingLevel={3} title="About this PC">
            <WSettingsValueList
              label="About this PC"
              onCopy={noop}
              items={[
                { key: "v", label: "Version", value: "0.9.81" },
                { key: "m", label: "Models folder", value: MODELS_PATH, mono: true, copy: true },
                { key: "d", label: "Device id", value: "example-device-0000-1111-2222-333344445555", mono: true, copy: true },
              ]}
            />
          </WSettingsCard>
        </Case>
      </Group>

      <Group shot="other" title="Settings parts: progress, disclosure, shortcut, notices">
        <Case label="Progress rows: known, unknown size, cancelling" wide>
          <WSettingsCard headingLevel={3} title="Models">
            <WSettingsProgressRow label="Transcription model" description="Downloading to this PC." progress={0.42} valueText="312 MB of 742 MB" onCancel={noop} />
            <WSettingsProgressRow label="Search model" progress={null} valueText="Preparing" onCancel={noop} />
            <WSettingsProgressRow label="Summary model" progress={0.9} valueText="668 MB of 742 MB" onCancel={noop} status="Cancelling…" />
          </WSettingsCard>
        </Case>
        <Case label="Disclosure: open, closed with a summary" wide>
          <WSettingsCard headingLevel={3} title="Summaries">
            <WSettingsDisclosure title="Instructions for each kind of note" summary="Using the defaults" open={open} onToggle={setOpen}>
              <WSettingsTextRow label="Meeting notes" value="" onCommit={noop} rows={2} />
              <WSettingsTextRow label="Call notes" value="" onCommit={noop} rows={2} />
            </WSettingsDisclosure>
            <WSettingsDisclosure title="Advanced" summary="Five settings">
              <WSettingsToggleRow label="Hidden" checked={false} onToggle={noop} />
            </WSettingsDisclosure>
          </WSettingsCard>
        </Case>
        <Case label="Shortcut rows: set, recording, not set" wide>
          <WSettingsCard headingLevel={3} title="Shortcuts">
            <WSettingsShortcutRow label="Start or stop notes" keys="Ctrl+Shift+Space" recording={recording} onRecord={() => setRecording(true)} onCancel={() => setRecording(false)} onClear={noop} />
            <WSettingsShortcutRow label="Open Whimbrel" keys={null} recording={false} onRecord={noop} onCancel={noop} />
            <WSettingsShortcutRow label="Pause memory" keys="Ctrl+Alt+P" recording onRecord={noop} onCancel={noop} error="That combination is used by Windows." />
          </WSettingsCard>
        </Case>
        <Case label="Notice box: info, ok, warning, error, with buttons" wide>
          <div className="g-stack">
            <WNoticeBox>{"Whimbrel works without any AI model: capture, transcripts and search still work."}</WNoticeBox>
            <WNoticeBox tone="ok" title="Saved">{"Your choices are kept on this PC."}</WNoticeBox>
            <WNoticeBox tone="warning" title="Restart needed" actions={<><WButton>{"Later"}</WButton><WButton variant="primary">{"Restart now"}</WButton></>}>{"Encryption finishes after Whimbrel restarts."}</WNoticeBox>
            <WNoticeBox tone="error">{"The memory database could not be opened."}</WNoticeBox>
          </div>
        </Case>
      </Group>

      <Group shot="dialogs" title="Settings parts: dialogs and confirms">
        <Case label="Typed confirm with a checkbox (focus starts in the box; confirm is off until DELETE is typed)" wide>
          <div className="g-dialog-host g-dialog-tall">
            {dialog === "typed" ? (
              <WConfirmDialog
                title="Delete everything on this PC?"
                body="This removes your meetings, memory, recordings and logs from this PC. It cannot be undone."
                confirmLabel="Delete everything"
                typed={{ word: "DELETE", label: "Type DELETE to go ahead", hint: "Capital letters." }}
                fields={<label className="w-check"><input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /><span>{"Also delete my encryption key"}</span></label>}
                onConfirm={() => setDialog("plain")}
                onCancel={() => setDialog("plain")}
              />
            ) : (
              <div className="g-pad"><WButton onClick={() => setDialog("typed")}>{"Open the dialog again"}</WButton></div>
            )}
          </div>
        </Case>
        <Case label="Plain destructive confirm (Cancel has focus)" wide>
          <div className="g-dialog-host">
            <WConfirmDialog title="Delete this skin?" body="Skins you made are kept only on this PC." confirmLabel="Delete skin" onConfirm={noop} onCancel={noop} />
          </div>
        </Case>
        <Case label="Inline confirm in a row, and the row's menu">
          <div className="g-stack">
            <WInlineConfirm question="Delete screen captures older than 30 days from this PC and your other devices? This cannot be undone." confirmLabel="Delete" onConfirm={noop} onCancel={noop} />
            <WMoreMenu items={[{ key: "a", label: "Rename", onPress: noop }]} />
          </div>
        </Case>
      </Group>

      <Group shot="skins" title="Settings parts: skin cards">
        <Case label="Skin cards: selected, unselected, yours with a menu (same height)" wide>
          <WSkinGrid label="Skins">
            {[
              { id: "whimbrel", name: "Whimbrel", caption: "Quiet and clear" },
              { id: "almanac", name: "Almanac", caption: "Paper and ink" },
              { id: "mine", name: "A skin with a long name that has to wrap", caption: "Made by you last week", yours: true },
              { id: "game-day", name: "Game day", caption: "Loud and bright" },
            ].map((s) => (
              <WSkinCard
                key={s.id}
                name={s.name}
                caption={s.caption}
                preview={prev}
                selected={skin === s.id}
                onSelect={() => setSkin(s.id)}
                yours={s.yours}
                menu={s.yours ? <WIconButton icon="more" label={`Options for ${s.name}`} onClick={noop} /> : undefined}
                palettes={s.id === "game-day" ? [palettes[0]] : palettes}
                paletteId={skin === s.id ? palette : undefined}
                onPalette={(id) => {
                  setSkin(s.id);
                  setPalette(id);
                }}
              />
            ))}
          </WSkinGrid>
        </Case>
      </Group>
    </main>
  );
}
