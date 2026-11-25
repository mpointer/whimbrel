import { useEffect, useState } from "react";
import { googleRemovalRevokes } from "../../../../api";
import { t } from "../../../../l10n";
import { WConfirmDialog } from "../../../../system";
import { removalMessage, type GoogleService } from "../../../RemoveGoogleAccount";

/**
 * The question before a Google account is removed (PR 798): nothing happens until Remove. It asks core whether the
 * removal also revokes Whimbrel's access with Google, and says which; Remove stays off until it knows. If the check
 * fails it says the cautious thing: access is kept.
 */
export function GoogleRemoveDialog({ service, email, busy, onRemove, onCancel }: { service: GoogleService; email: string; busy: boolean; onRemove: () => void; onCancel: () => void }) {
  const [revokes, setRevokes] = useState<boolean | null>(null);
  useEffect(() => {
    let live = true;
    googleRemovalRevokes(service, email).then(
      (r) => live && setRevokes(r),
      () => live && setRevokes(false),
    );
    return () => {
      live = false;
    };
  }, [service, email]);
  return (
    <WConfirmDialog
      title={t("Remove this Google account?")}
      body={revokes != null ? removalMessage(service, revokes) : undefined}
      fields={<p>{email}</p>}
      confirmLabel={t("Remove")}
      confirmDisabled={revokes == null}
      busy={busy}
      onConfirm={onRemove}
      onCancel={onCancel}
    />
  );
}
