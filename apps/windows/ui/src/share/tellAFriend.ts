// Tell a friend (docs/superpowers/plans/2026-11-24-referral-and-share.md, rollout row 1): a quiet row in the tray popup and a card
// in Settings, Sync & Devices, and an item in the tray's right-click menu (Rust). Each copies a short message with a plain link
// to whimbrelapp.com. No account is needed, no code and no reward is part of the message, nothing opens and nothing is sent.

import { useEffect, useRef, useState } from "react";
import { t as translate, type Args } from "../l10n";
import { copyToClipboard } from "../system";

/** The plain link: the home page with the one campaign tag the site already keeps in the visitor's own browser. The Rust side
 *  (`tell_friend.rs`) holds the same link, and a test there and a test here pin them to each other. */
export const TELL_FRIEND_LINK = "https://whimbrelapp.com/?utm_source=share";

/** How long "Copied" shows before the row or button goes back to its name. */
export const COPIED_MS = 4000;

type Translate = (key: string, args?: Args) => string;

/** The message a person copies. A translation that lost its `{link}` still gives a message with the link in it once, at the end,
 *  so a copy never loses the link. */
export function tellAFriendMessage(t: Translate = translate, link: string = TELL_FRIEND_LINK): string {
  const text = t("I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: {link}", { link });
  return text.includes(link) ? text : `${text.replace(/\{link\}/g, "").trim()} ${link}`.trim();
}

/** The copy and its "Copied" flash. `copied` is true for a few seconds after a copy that worked, `failed` after one that did not. */
export function useTellAFriend(t: Translate = translate) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const copy = () => {
    void copyToClipboard(tellAFriendMessage(t)).then((ok) => {
      if (timer.current) clearTimeout(timer.current);
      setState(ok ? "copied" : "failed");
      timer.current = setTimeout(() => setState("idle"), COPIED_MS);
    });
  };
  return { copied: state === "copied", failed: state === "failed", copy };
}
