/**
 * The meeting alert's one dismissing control. With no countdown it is Not now, as it has always been. While the auto-start
 * countdown runs it is Don't record, in the place of Not now (the Mac's words, AlertPanelsRedesign.swift): both end in
 * `alert_dismiss`, which closes the alert, starts nothing and, because the detector prompts once per app and the calendar
 * list remembers the event, does not offer this meeting again. There is no separate Cancel.
 */
export function dismissWords(counting: boolean, t: (key: string) => string): { label: string; help: string | undefined } {
  if (!counting) return { label: t("Not now"), help: undefined };
  return {
    label: t("Don’t record"),
    help: t("Stops the countdown. Whimbrel will not ask about this meeting again."),
  };
}
