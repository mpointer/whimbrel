// Settings → General's language pickers (docs/superpowers/plans/2026-10-31-windows-localization.md, W3),
// as the Mac's General pane has them. Languages are named in the language the app shows.

import type { LanguageSettings } from "../generated/types";
import { t } from "../l10n";

export interface Option {
  value: string;
  label: string;
}

/** `tag`'s name in `displayIn` ("en-GB" → "British English"), or the tag if Intl has none. */
export function languageName(tag: string, displayIn: string): string {
  try {
    return new Intl.DisplayNames([displayIn], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/** Automatic, then each language this build ships. */
export function languageOptions(s: LanguageSettings): Option[] {
  return [{ value: "", label: t("Automatic") }, ...s.languages.map((tag) => ({ value: tag, label: languageName(tag, s.uiLanguage) }))];
}

/** "Same as app language", then the Mac's six writing languages. */
export function writeInOptions(s: LanguageSettings): Option[] {
  return [
    { value: "", label: t("Same as app language") },
    ...s.writeInOptions.map((tag) => ({ value: tag, label: languageName(tag, s.uiLanguage) })),
  ];
}

/** Under the writing picker when a local model will be asked to write in another language. */
export function localModelNote(s: LanguageSettings): string | null {
  return s.localModelNote ? t("Local models may write less well in {language}.", { language: languageName(s.outputLanguage, s.uiLanguage) }) : null;
}
