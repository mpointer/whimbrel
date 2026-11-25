import { useEffect, useState } from "react";
import { googleClientClear, googleClientSave, googleClientStatus } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { GoogleClientStatus } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WSettingsCard, WSettingsDisclosure, WSettingsSecretRow, WSettingsTextRow } from "../../../../system";
import { GOOGLE_CLIENT_CHANGED } from "../../../googleClientModel";
import { useJump } from "../../jump";
import { clientIdChangedHint, ownClientReason } from "./connectorsModel";
import "./connectors.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Advanced: "Use your own Google Cloud project". Public builds ask Google only to send mail; a Desktop OAuth client
 * from the user's own project lets Gmail read too. The client ID and the secret are saved together. The secret goes
 * to Credential Manager and is never drawn back: once saved, the row says so and offers Replace and Remove.
 */
export function OwnClientDisclosure() {
  const jumped = useJump("connectors.ownClient");
  const [status, setStatus] = useState<GoogleClientStatus | null>(null);
  const [clientId, setClientId] = useState("");
  // Bumped after each save or clear, so the secret row starts again with nothing typed in it.
  const [round, setRound] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<boolean | null>(null);

  useEffect(() => {
    googleClientStatus()
      .then((s) => {
        setStatus(s);
        setClientId(s.clientId ?? "");
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const saved = status?.clientId ?? null;
  // Open while a client is saved, as the old card did, and when a search result is for this card.
  const isOpen = open ?? saved != null;
  // A search result for this card opens it and leaves it open (the person can still close it).
  useEffect(() => {
    if (jumped) setOpen(true);
  }, [jumped]);

  const run = (call: () => Promise<GoogleClientStatus>): Promise<boolean> => {
    setBusy(true);
    setError(null);
    return call()
      .then((s) => {
        setStatus(s);
        setClientId(s.clientId ?? "");
        setRound((r) => r + 1);
        // Connected Google accounts now need signing in again; their cards reload to say so.
        window.dispatchEvent(new Event(GOOGLE_CLIENT_CHANGED));
        return true;
      })
      .catch((e: unknown) => {
        setError(message(e));
        return false;
      })
      .finally(() => setBusy(false));
  };

  return (
    <WSettingsDisclosure title={t("Advanced")} open={isOpen} onToggle={setOpen}>
      <WSettingsCard
        id="connectors.ownClient"
        title={t("Use your own Google Cloud project")}
        headingLevel={3}
        description={t(
          "For Gmail reading. Create a Desktop OAuth client in your own Google Cloud project, add yourself as its user, and publish it (an unverified app is fine for yourself).",
        )}
        error={error}
      >
        <WSettingsTextRow label={t("Client ID")} value={clientId} mono required requiredMessage={ownClientReason("") ?? undefined} disabled={busy} onCommit={setClientId} />
        <WSettingsSecretRow
          key={round}
          label={t("Client secret")}
          saved={saved != null}
          savedText={t("Saved")}
          busy={busy}
          disabled={saved == null && ownClientReason(clientId) !== null}
          disabledReason={ownClientReason(clientId) ?? undefined}
          hint={saved != null && clientId.trim() !== saved ? clientIdChangedHint() : undefined}
          onSave={(secret) => {
            const why = ownClientReason(clientId);
            if (why) {
              setError(why);
              return false;
            }
            return run(() => googleClientSave(clientId, secret));
          }}
          removeQuestion={t("Remove your own Google client? Every connected Google account is signed out and has to be connected again.")}
          onRemove={() => void run(googleClientClear)}
        />
      </WSettingsCard>
    </WSettingsDisclosure>
  );
}
