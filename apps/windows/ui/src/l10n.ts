// The UI's text, from the catalog the Mac's String Catalogs share a format with
// (docs/superpowers/plans/2026-10-31-windows-localization.md). Keys are the English source text,
// with {name} placeholders; a missing translation shows the English, never the key's absence.
// `translate.py extract` collects every t() call with a string literal into the catalog (comments
// too, so don't write one in a comment), so keys stay literals.

import { createContext, createElement, useContext, useSyncExternalStore, type ReactNode } from "react";
// The file name is fixed by translate.py (WINDOWS_CATALOG), so it's read as text, not imported as JSON.
import catalogText from "./l10n/Localizable.xcstrings?raw";

interface StringUnit {
  stringUnit?: { value: string };
}

interface Localization extends StringUnit {
  variations?: { plural?: Record<string, StringUnit> };
}

export interface Catalog {
  strings: Record<string, { localizations?: Record<string, Localization> }>;
}

export type Args = Record<string, string | number>;

const BUNDLED: Catalog = JSON.parse(catalogText);

/** The base language of a tag: `fr-CA` → `fr`. */
function base(lang: string): string {
  return lang.split("-")[0];
}

/** One language's form of `key`, or null when that language has none. A plural entry picks by
 *  `count` under that language's own rules (French and Portuguese zero is "one", as in core). */
function form(entry: Localization | undefined, lang: string, count: number | undefined): string | null {
  if (!entry) return null;
  const plural = entry.variations?.plural;
  if (plural && count !== undefined) {
    const category = new Intl.PluralRules(lang).select(count);
    return plural[category]?.stringUnit?.value ?? plural.other?.stringUnit?.value ?? null;
  }
  return entry.stringUnit?.value ?? null;
}

/** A number as `lang` writes it ("1,487", "1 487"), as the Mac's `%lld` placeholders do. */
function formatNumber(n: number, lang: string): string {
  try {
    return new Intl.NumberFormat(lang === "en-XA" ? "en" : lang).format(n);
  } catch {
    return String(n);
  }
}

/** The text for `key` in `lang`: the exact tag, then the base language, then English, then the key
 *  itself, which is the English source. Placeholders are filled from `args`; numbers are formatted
 *  for `lang`. */
export function lookup(catalog: Catalog, lang: string, key: string, args?: Args): string {
  const count = typeof args?.count === "number" ? args.count : undefined;
  const localizations = catalog.strings[key]?.localizations ?? {};
  // English falls back under English rules: "0 meetings", even when the language asked for was French.
  const text =
    form(localizations[lang], lang, count) ??
    form(localizations[base(lang)], base(lang), count) ??
    form(localizations.en, "en", count) ??
    key;
  if (!args) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => {
    if (!(name in args)) return whole;
    const value = args[name];
    return typeof value === "number" ? formatNumber(value, lang) : value;
  });
}

/** The language to show. `en-XA` (the pseudo-locale) is a development check only. */
export function resolveLanguage(tag: string): string {
  return tag === "en-XA" && !import.meta.env.DEV ? "en" : tag;
}

// Module-level, so the .ts models (tab labels, status lines) can call t() outside a component.
// The window's root reads it through useLanguage() and re-renders when setLanguage changes it.
let current = "en";
const listeners = new Set<() => void>();

/** Shows `lang` from now on: t() uses it at once, and useLanguage() re-renders its component. */
export function setLanguage(lang: string): void {
  const next = resolveLanguage(lang);
  if (next === current) return;
  current = next;
  listeners.forEach((listener) => listener());
}

/** Calls `listener` after each change of language; returns the unsubscribe. */
export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The current language, for a window's root to hand to L10nProvider. */
export function useLanguage(): string {
  return useSyncExternalStore(subscribeLanguage, () => current);
}

/** `key` in the current language. */
export function t(key: string, args?: Args): string {
  return lookup(BUNDLED, current, key, args);
}

export interface L10n {
  lang: string;
  /** The tag Intl formatters take: the pseudo-locale formats dates and numbers as English. */
  locale: string;
  t: typeof t;
}

// One value per language, so a re-render with the same language doesn't re-render every consumer.
const byLang = new Map<string, L10n>();

function l10nFor(lang: string): L10n {
  let l10n = byLang.get(lang);
  if (!l10n) {
    l10n = { lang, locale: lang === "en-XA" ? "en" : lang, t: (key, args) => lookup(BUNDLED, lang, key, args) };
    byLang.set(lang, l10n);
  }
  return l10n;
}

const L10nContext = createContext<L10n>(l10nFor("en"));

/** The language for useL10n() below it. A window's root passes useLanguage(), so t() agrees. */
export function L10nProvider({ lang, children }: { lang: string; children: ReactNode }) {
  return createElement(L10nContext.Provider, { value: l10nFor(resolveLanguage(lang)) }, children);
}

export function useL10n(): L10n {
  return useContext(L10nContext);
}
