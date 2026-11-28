import { useEffect, useState } from "react";
import { runningEmulated } from "../api";
import { t } from "../l10n";
import { WBanner, WButton } from "../system";

const DISMISSED = "whimbrel.armNotice.dismissed";

function dismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED) === "1";
  } catch {
    return false;
  }
}

/** Says when this x64 build runs under emulation on an Arm PC, where everything heavy is slower and
 *  calls can stutter while a meeting records (a tester's laptop, 2026-11-13). Shown until dismissed. */
export function ArmNotice() {
  const [show, setShow] = useState(false);

  useEffect(() => {
    if (dismissed()) return;
    runningEmulated()
      .then(setShow)
      .catch(() => {});
  }, []);

  if (!show) return null;
  const dismiss = () => {
    try {
      localStorage.setItem(DISMISSED, "1");
    } catch {
      // Shown again next launch.
    }
    setShow(false);
  };
  const line = t(
    "This PC has an Arm processor, and this version of Whimbrel is built for Intel and AMD processors, so Windows runs it through emulation. Transcription and summaries take longer and use more of the PC, and video calls may stutter while a meeting records. Choosing Whimbrel AI in Settings → AI moves summaries off this PC.",
  );
  return (
    <WBanner label={t("Arm PC")} actions={<WButton onClick={dismiss}>{t("Dismiss")}</WButton>}>
      <p>{line}</p>
    </WBanner>
  );
}
