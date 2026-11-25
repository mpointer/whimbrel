import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WSettingsRow, WSettingsSearchResults, WSettingsSecretRow, WSettingsValueList, WStatusChip } from ".";

const html = (n: React.ReactNode) => renderToStaticMarkup(<>{n}</>);

describe("the secret row, drawn", () => {
  it("saved: a masked line with Replace and Remove, no box, and no value in the page", () => {
    const out = html(<WSettingsSecretRow label="Example key" saved onSave={() => true} onRemove={() => {}} />);
    expect(out).not.toContain('type="password"');
    expect(out).not.toContain("<input");
    expect(out).toContain("Saved");
    expect(out).toContain("Replace");
    expect(out).toContain("Remove");
  });

});

describe("search results", () => {
  it("shows each setting with its section, or says there is none", () => {
    const out = html(<WSettingsSearchResults results={[{ id: "x", title: "Keep screen text for", section: "Storage" }]} onOpen={() => {}} />);
    expect(out).toContain("Keep screen text for");
    expect(out).toContain("Storage");
    expect(html(<WSettingsSearchResults results={[]} onOpen={() => {}} />)).toContain("No matching settings");
  });
});

describe("rows and parts, drawn", () => {
  it("a disabled row shows its reason in words and ties it to the control", () => {
    const out = html(<WSettingsRow variant="toggle" label="Start with Windows" checked={false} onToggle={() => {}} disabled disabledReason="Available in release builds only." />);
    expect(out).toContain("Available in release builds only.");
    expect(out).toMatch(/disabled=""/);
    expect(out).toMatch(/aria-describedby="[^"]+"/);
  });

  it("a row that is not disabled does not show a reason", () => {
    expect(html(<WSettingsRow variant="toggle" label="X" checked onToggle={() => {}} disabledReason="Nope." />)).not.toContain("Nope.");
  });

  it("the checkbox variant is a native checkbox inside its label, with the note", () => {
    const out = html(<WSettingsRow variant="checkbox" label="Show captures" description="Off by default." checked={false} onToggle={() => {}} />);
    expect(out).toContain('type="checkbox"');
    expect(out).toContain("Off by default.");
  });

  it("a highlighted row can take focus and is marked", () => {
    const out = html(<WSettingsRow variant="toggle" id="row-x" label="X" checked onToggle={() => {}} highlighted />);
    expect(out).toContain('id="row-x"');
    expect(out).toContain('tabindex="-1"');
    expect(out).toContain("w-setting-jumped");
  });

  it("a status chip says the state in words with a glyph, for every tone", () => {
    for (const tone of ["neutral", "good", "warning", "problem"] as const) {
      const out = html(<WStatusChip tone={tone}>State words</WStatusChip>);
      expect(out).toContain("State words");
      expect(out).toContain("<svg");
    }
  });

  it("a value list is a description list, with long values allowed to wrap", () => {
    const out = html(<WSettingsValueList label="About" items={[{ key: "p", label: "Path", value: "C:\\a\\b", mono: true, copy: true }]} />);
    expect(out).toContain("<dl");
    expect(out).toContain("<dt>Path</dt>");
    expect(out).toContain('aria-label="Copy Path"');
  });
});
