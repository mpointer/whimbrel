import { useEffect, useState } from "react";
import { googleRemovalRevokes } from "../api";
import { t } from "../l10n";

export type GoogleService = "gmail" | "calendar";

/** What removing the account does, as the confirmation says it (the Mac's `GoogleDisconnect`).
 *  `revokes` is false only while another Whimbrel connection on this PC uses the address. */
export function removalMessage(service: GoogleService, revokes: boolean): string {
  if (service === "gmail") {
    return revokes
      ? t("This removes this account's Gmail data from this PC and revokes Whimbrel's access to the account with Google.")
      : t("This removes this account's Gmail data from this PC. Whimbrel keeps its Google access to this address while another Whimbrel connection to it remains.");
  }
  return revokes
    ? t("This removes this account's Calendar events from this PC and revokes Whimbrel's access to the account with Google. Meetings you recorded are kept.")
    : t("This removes this account's Calendar events from this PC. Whimbrel keeps its Google access to this address while another Whimbrel connection to it remains. Meetings you recorded are kept.");
}

/** The confirmation before a Google account is removed (PR 798): nothing happens until Remove. */
export function RemoveGoogleAccount({
  service,
  email,
  busy,
  onRemove,
  onCancel,
}: {
  service: GoogleService;
  email: string;
  busy: boolean;
  onRemove: () => void;
  onCancel: () => void;
}) {
  const [revokes, setRevokes] = useState<boolean | null>(null);
  useEffect(() => {
    let live = true;
    // If the check fails, say the cautious thing: access is kept.
    googleRemovalRevokes(service, email).then((r) => live && setRevokes(r), () => live && setRevokes(false));
    return () => {
      live = false;
    };
  }, [service, email]);

  return (
    <span role="alertdialog" aria-label={t("Remove this Google account?")}>
      <span className="row-title">{t("Remove this Google account?")}</span>{" "}
      {revokes != null && <span className="muted small">{removalMessage(service, revokes)}</span>}
      <button className="mode mode--active" disabled={busy || revokes == null} onClick={onRemove}>
        {t("Remove")}
      </button>
      <button className="mode" disabled={busy} onClick={onCancel}>
        {t("Cancel")}
      </button>
    </span>
  );
}
