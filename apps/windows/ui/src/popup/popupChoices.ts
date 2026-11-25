import { t } from "../l10n";
import type { PauseChoice } from "../shell/redesign/captureModel";

/** The short words of a timed choice, under the group name "Pause memory". Literals, so the string extractor finds each one. */
export function shortPauseLabel(choice: PauseChoice): string {
  switch (choice) {
    case "15m":
      return t("15 minutes");
    case "1h":
      return t("1 hour");
    case "tomorrow":
      return t("Until tomorrow");
  }
}
