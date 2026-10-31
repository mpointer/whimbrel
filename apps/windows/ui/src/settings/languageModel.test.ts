import { describe, expect, it } from "vitest";
import type { LanguageSettings } from "../generated/types";
import { languageName, languageOptions, localModelNote, writeInOptions } from "./languageModel";

const settings: LanguageSettings = {
  choice: "",
  uiLanguage: "en-US",
  languages: ["en", "en-GB", "en-CA", "en-AU"],
  writeIn: "",
  writeInOptions: ["en-US", "en-GB", "en-CA", "en-AU", "es-US", "fr-CA"],
  outputLanguage: "",
  localModelNote: false,
};

describe("language pickers", () => {
  it("offer Automatic, then each shipped language by name", () => {
    expect(languageOptions(settings).map((o) => o.label)).toEqual([
      "Automatic",
      "English",
      "British English",
      "Canadian English",
      "Australian English",
    ]);
    expect(languageOptions(settings)[0].value).toBe("");
  });

  it("offer Same as app language, then the Mac's six writing languages", () => {
    const options = writeInOptions(settings);
    expect(options[0]).toEqual({ value: "", label: "Same as app language" });
    expect(options.map((o) => o.value).slice(1)).toEqual(settings.writeInOptions);
    expect(options.find((o) => o.value === "fr-CA")?.label).toBe("Canadian French");
  });

  it("name a language in the one the app shows", () => {
    expect(languageName("es-US", "en-GB")).toMatch(/Spanish/);
    expect(languageName("not a tag!", "en")).toBe("not a tag!");
  });

  it("note a local model only when Rust says so", () => {
    expect(localModelNote(settings)).toBeNull();
    expect(localModelNote({ ...settings, outputLanguage: "es-US", localModelNote: true })).toMatch(
      /^Local models may write less well in .*Spanish.*\.$/,
    );
  });
});
