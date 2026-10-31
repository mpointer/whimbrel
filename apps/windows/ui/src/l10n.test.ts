import { describe, expect, it } from "vitest";
import { lookup, resolveLanguage, setLanguage, subscribeLanguage, t, type Catalog } from "./l10n";

const unit = (value: string) => ({ stringUnit: { value } });

const CATALOG: Catalog = {
  strings: {
    Meetings: { localizations: { fr: unit("Réunions"), "fr-CA": unit("Rencontres") } },
    "{count} meetings": {
      localizations: {
        en: { variations: { plural: { one: unit("{count} meeting"), other: unit("{count} meetings") } } },
        fr: { variations: { plural: { one: unit("{count} réunion"), other: unit("{count} réunions") } } },
      },
    },
    "Hello, {name}": { localizations: { es: unit("Hola, {name}") } },
  },
};

describe("lookup", () => {
  it("t falls back exact, base, english", () => {
    expect(lookup(CATALOG, "fr-CA", "Meetings")).toBe("Rencontres");
    expect(lookup(CATALOG, "fr-FR", "Meetings")).toBe("Réunions");
    expect(lookup(CATALOG, "de", "Meetings")).toBe("Meetings");
  });

  it("fr zero uses one", () => {
    expect(lookup(CATALOG, "fr-CA", "{count} meetings", { count: 0 })).toBe("0 réunion");
    expect(lookup(CATALOG, "fr", "{count} meetings", { count: 2 })).toBe("2 réunions");
  });

  it("english plural one comes from the catalog", () => {
    expect(lookup(CATALOG, "en", "{count} meetings", { count: 1 })).toBe("1 meeting");
    expect(lookup(CATALOG, "en", "{count} meetings", { count: 0 })).toBe("0 meetings");
    // A fallback to English uses English rules, not the asked-for language's.
    expect(lookup(CATALOG, "de", "{count} meetings", { count: 0 })).toBe("0 meetings");
  });

  it("interpolates {name}", () => {
    expect(lookup(CATALOG, "es-US", "Hello, {name}", { name: "Ana" })).toBe("Hola, Ana");
    expect(lookup(CATALOG, "es", "Hello, {name}", {})).toBe("Hola, {name}");
  });

  it("formats numbers for the language", () => {
    expect(lookup(CATALOG, "en", "{count} meetings", { count: 1487 })).toBe("1,487 meetings");
    expect(lookup(CATALOG, "fr", "{count} meetings", { count: 2 })).toBe("2 réunions");
  });

  it("missing key returns english", () => {
    expect(lookup(CATALOG, "fr", "Not in the catalog {x}", { x: 1 })).toBe("Not in the catalog 1");
  });
});

describe("t", () => {
  it("shows English from the bundled catalog until a language is set", () => {
    // Not a literal: translate.py extract would collect a literal key from a test into the catalog.
    const key = "Settings";
    expect(t(key)).toBe("Settings");
  });
});

describe("resolveLanguage", () => {
  it("keeps the pseudo-locale in development builds, which Vitest is", () => {
    expect(import.meta.env.DEV).toBe(true);
    expect(resolveLanguage("en-XA")).toBe("en-XA");
    expect(resolveLanguage("fr-CA")).toBe("fr-CA");
  });
});

describe("setLanguage", () => {
  // useLanguage() is useSyncExternalStore over subscribeLanguage, so a notified listener is a re-render.
  it("setLanguage re-renders consumers", () => {
    let renders = 0;
    const stop = subscribeLanguage(() => renders++);
    setLanguage("en-GB");
    setLanguage("en-GB");
    // Not a literal: translate.py extract would collect a literal key from a test into the catalog.
    const key = "Settings";
    expect([renders, t(key)]).toEqual([1, "Settings"]);
    setLanguage("en");
    stop();
    setLanguage("en-GB");
    expect(renders).toBe(2);
    setLanguage("en");
  });
});
