import { t } from "../../l10n";
import type { PauseChoice } from "./captureModel";

/** The words of a timed pause choice. Literals, so the string extractor finds each one. */
export function pauseChoiceLabel(choice: PauseChoice): string {
  switch (choice) {
    case "15m":
      return t("Pause for 15 minutes");
    case "1h":
      return t("Pause for 1 hour");
    case "tomorrow":
      return t("Pause until tomorrow");
  }
}
