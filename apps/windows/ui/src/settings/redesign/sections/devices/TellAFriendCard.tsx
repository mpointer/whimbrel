import { t } from "../../../../l10n";
import { useTellAFriend } from "../../../../share/tellAFriend";
import { WSettingsCard, WSettingsRow } from "../../../../system";

/**
 * Tell a friend: copies a short message with a plain link to whimbrelapp.com. No account is needed and nothing is sent from
 * here; the person pastes it wherever they like. "Copied" shows on the button for a few seconds and is said once.
 */
export function TellAFriendCard() {
  const share = useTellAFriend();
  return (
    <WSettingsCard
      id="devices.tellAFriend"
      title={t("Tell a friend")}
      headingLevel={3}
      description={t("Copy a short message with a link to Whimbrel, then paste it wherever you like. Nothing is sent from here.")}
      error={share.failed ? t("Could not copy. Try again.") : null}
    >
      <WSettingsRow variant="button" label={t("Copy message")} actionLabel={share.copied ? t("Copied") : t("Copy")} onPress={share.copy} />
      <span className="w-visually-hidden" role="status">
        {share.copied ? t("Copied") : ""}
      </span>
    </WSettingsCard>
  );
}
