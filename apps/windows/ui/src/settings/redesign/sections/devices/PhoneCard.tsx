import { useEffect, useState } from "react";
import { openExternal, syncDevices } from "../../../../api";
import betaQr from "../../../../assets/mobile-beta-qr.svg";
import type { DeviceView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WSettingsCard, WSettingsLinkRow } from "../../../../system";
import { phoneCardLoading, phoneCardState } from "../../../syncModel";
import "./devices.css";

/** The phone app's beta sign-up. The backend opens only pages on its allowlist, and this is one. */
const MOBILE_BETA_URL = "https://whimbrelapp.com/mobile-beta/";

const PHONE_WAIT_MS = 3000;

/**
 * Whimbrel on your phone: the beta card until an approved phone is on the account, then the same card saying so.
 * The device list is read once when the section opens with sync ready (a call to the sync server).
 */
export function PhoneCard({ ready }: { ready: boolean }) {
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [waitedOut, setWaitedOut] = useState(false);
  useEffect(() => {
    if (!ready) return;
    void syncDevices().then(
      (d) => {
        setDevices(d);
        setLoaded(true);
      },
      () => {},
    );
    // If the list never comes (offline), show the beta card after a few seconds.
    const timer = setTimeout(() => setWaitedOut(true), PHONE_WAIT_MS);
    return () => clearTimeout(timer);
  }, [ready]);
  if (phoneCardLoading(ready, loaded, waitedOut)) return null;
  if (phoneCardState(devices) === "connected") {
    return <WSettingsCard id="devices.phone" title={t("Whimbrel on your phone")} headingLevel={3} status={{ tone: "good", children: t("Phone: connected") }} />;
  }
  return (
    <WSettingsCard
      id="devices.phone"
      title={t("Whimbrel on your phone")}
      headingLevel={3}
      description={t("Read your meeting notes, decisions and action items on iPhone or Android. The apps are in beta.")}
    >
      <WSettingsLinkRow kind="external" label={t("Join the beta")} href={MOBILE_BETA_URL} onPress={() => void openExternal(MOBILE_BETA_URL)} />
      <div className="rs-devconn-phone">
        <figure className="rs-devconn-qr">
          <img src={betaQr} width={84} height={84} alt={t("QR code for the phone beta page")} />
          <figcaption>{t("Scan with your phone")}</figcaption>
        </figure>
      </div>
    </WSettingsCard>
  );
}
