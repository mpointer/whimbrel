import { useEffect, useState, useSyncExternalStore } from "react";
import { mcpRevokeClient } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import { t } from "../../../../l10n";
import { WButton, WDialog, WInlineConfirm, WNoticeBox, copyToClipboard } from "../../../../system";
import { mcpTokenOnce } from "../../../mcpModel";
import { mcpToken } from "./mcpTokenStore";
import "../devices/devices.css";
import "./connectors.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/**
 * The new-token dialog, and the notice that follows a token nobody copied. The frame draws this above the section
 * switch, so the dialog stays while the person goes to another section; it is the tab going away that drops the token.
 */
export function McpTokenHost() {
  useSyncExternalStore(mcpToken.subscribe, mcpToken.version, mcpToken.version);
  const made = mcpToken.get();
  const orphan = mcpToken.orphan();
  const [copyFailed, setCopyFailed] = useState(false);
  // Escape or Done before the token was copied asks first: the token is not shown again.
  const [askClose, setAskClose] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [revokeError, setRevokeError] = useState<string | null>(null);

  // The tab going away is the end of the token: it is dropped, and a notice is left if it was never copied.
  useEffect(() => () => mcpToken.abandon(), []);

  const close = () => {
    setCopyFailed(false);
    setAskClose(false);
    mcpToken.clear();
    // The button that made the token is off again (the name is empty), so focus goes to the name box when it is there.
    setTimeout(() => document.getElementById("rs-mcp-name")?.focus(), 0);
  };
  /** Escape and Done: close at once only after the token was copied; before that, ask. */
  const tryClose = () => (made?.copied ? close() : setAskClose(true));
  const copyToken = (token: string) =>
    void copyToClipboard(token).then((ok) => {
      mcpToken.markCopied(ok);
      setCopyFailed(!ok);
    });

  const revoke = (id: number) => {
    setRevoking(true);
    setRevokeError(null);
    mcpRevokeClient(id).then(
      () => {
        setRevoking(false);
        mcpToken.clearOrphan();
      },
      (e: unknown) => {
        setRevoking(false);
        setRevokeError(message(e));
      },
    );
  };

  return (
    <>
      {orphan && !made && (
        <WNoticeBox
          tone="warning"
          live
          actions={
            <WButton variant="primary" disabled={revoking} onClick={() => revoke(orphan.id)}>
              {t("Revoke")}
            </WButton>
          }
        >
          <p>{t("A token was created for {name} but was not copied. Revoke it and create a new one.", { name: orphan.name })}</p>
          {revokeError && <p>{revokeError}</p>}
        </WNoticeBox>
      )}
      {made && (
        <WDialog
          title={t("Your new token")}
          // The only-time sentence is the dialog's description, so it is read with the title.
          body={mcpTokenOnce()}
          onCancel={tryClose}
          details={
            <>
              <p className="rs-devconn-dialog-text">
                <strong>{t("Token for {name}", { name: made.name })}</strong>
              </p>
              <code className="rs-devconn-token" data-testid="mcp-token">
                {made.token}
              </code>
              <div role="status">{made.copied && <p className="rs-devconn-dialog-note">{t("Copied")}</p>}</div>
              <div role="alert">{copyFailed && <p className="rs-devconn-dialog-note">{t("Could not copy. Select the token and copy it yourself.")}</p>}</div>
              {askClose && (
                <WInlineConfirm
                  question={t("Close without copying? This token is not shown again.")}
                  confirmLabel={t("Close without copying")}
                  cancelLabel={t("Keep open")}
                  onConfirm={close}
                  onCancel={() => setAskClose(false)}
                />
              )}
            </>
          }
        >
          <WButton data-initial onClick={() => copyToken(made.token)}>
            {t("Copy")}
          </WButton>
          <WButton variant="primary" onClick={tryClose}>
            {t("Done")}
          </WButton>
        </WDialog>
      )}
    </>
  );
}
