import { useState } from "react";
import type { InviteeView, TranscriptTurn } from "../generated/types";
import { useL10n } from "../l10n";
import { speakerLabel } from "./sentinels";

/** What the speaker menu lists, in order (the Mac's `speakerMenuItems`). */
export type SpeakerMenuItem =
  | { kind: "confirm"; name: string; personKey: string }
  | { kind: "invitee"; invitee: InviteeView }
  | { kind: "someoneElse" };

/** "Yes, that's {name}" for a guess first, then each invitee, then "Someone else…". */
export function speakerMenuItems(turn: TranscriptTurn, invitees: InviteeView[]): SpeakerMenuItem[] {
  const items: SpeakerMenuItem[] = [];
  if (turn.guess && turn.personKey) items.push({ kind: "confirm", name: turn.label, personKey: turn.personKey });
  for (const invitee of invitees) items.push({ kind: "invitee", invitee });
  items.push({ kind: "someoneElse" });
  return items;
}

/** A turn's speaker as shown: a name, a guess as "Name?", or the stored label translated. */
export function shownSpeaker(turn: TranscriptTurn): string {
  if (turn.label === turn.speaker) return speakerLabel(turn.speaker);
  return turn.guess ? `${turn.label}?` : turn.label;
}

/**
 * A transcript row's speaker, and when `offered` a menu to confirm or fix it. A choice applies
 * to every line of that speaker in the meeting.
 */
export function SpeakerMenu({
  turn,
  invitees,
  offered,
  onConfirm,
  onPick,
}: {
  turn: TranscriptTurn;
  invitees: InviteeView[];
  offered: boolean;
  onConfirm: (personKey: string) => void;
  onPick: (name: string, email: string | null) => void;
}) {
  const { t } = useL10n();
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState<string | null>(null);
  const className = turn.guess ? "transcript-speaker transcript-speaker--guess" : "transcript-speaker";
  if (!offered) return <span className={className}>{shownSpeaker(turn)}</span>;

  const saveTyped = () => {
    const name = (typed ?? "").trim();
    if (!name) return;
    setTyped(null);
    onPick(name, null);
  };

  return (
    <span className="speaker-menu">
      <button className={`link ${className}`} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen(!open)}>
        {shownSpeaker(turn)}
      </button>
      {open && (
        <span role="menu" className="speaker-menu-list">
          {speakerMenuItems(turn, invitees).map((item, i) => {
            const choose = (run: () => void) => () => {
              setOpen(false);
              run();
            };
            switch (item.kind) {
              case "confirm":
                return (
                  <button key={i} role="menuitem" className="mode" onClick={choose(() => onConfirm(item.personKey))}>
                    {t("Yes, that's {name}", { name: item.name })}
                  </button>
                );
              case "invitee":
                return (
                  <button
                    key={i}
                    role="menuitem"
                    className="mode"
                    onClick={choose(() => onPick(item.invitee.name, item.invitee.email ?? null))}
                  >
                    {item.invitee.name}
                  </button>
                );
              case "someoneElse":
                return (
                  <button key={i} role="menuitem" className="mode" onClick={choose(() => setTyped(""))}>
                    {t("Someone else…")}
                  </button>
                );
            }
          })}
        </span>
      )}
      {typed != null && (
        <span className="speaker-menu-typed">
          <input
            className="input"
            aria-label={t("Name")}
            autoFocus
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && saveTyped()}
          />
          <button className="mode mode--active" disabled={!typed.trim()} onClick={saveTyped}>
            {t("Save")}
          </button>
          <button className="mode" onClick={() => setTyped(null)}>
            {t("Cancel")}
          </button>
        </span>
      )}
    </span>
  );
}
