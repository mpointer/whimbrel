import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { LibraryMissingBannerView, RESTORE_HELP_URL } from "./LibraryMissingBanner";
import { confirmLine, keptSize, resultLines, showCard, summaryLine } from "./foreignFilesModel";

describe("files kept aside (X4 Task 13c item 3)", () => {
  it("shows the card only while some exist", () => {
    expect(showCard(null)).toBe(false);
    expect(showCard({ count: 0, bytes: 0 })).toBe(false);
    expect(showCard({ count: 2, bytes: 120 })).toBe(true);
  });

  it("names the count and size, singular and plural, and says the delete is final", () => {
    expect(summaryLine({ count: 1, bytes: 2_000_000 })).toBe("1 file kept aside, 2.0 MB");
    expect(summaryLine({ count: 3, bytes: 12_000_000 })).toBe("3 files kept aside, 12.0 MB");
    expect(confirmLine({ count: 1, bytes: 2_000_000 })).toBe("This deletes 1 file kept aside (2.0 MB). It cannot be recovered.");
    expect(confirmLine({ count: 3, bytes: 12_000_000 })).toBe("This deletes 3 files kept aside (12.0 MB). They cannot be recovered.");
  });

  it("says what was deleted, and names the ones that stayed", () => {
    expect(resultLines({ removed: 2, removedBytes: 2_500_000, failed: [] })).toEqual({ done: ["Deleted 2 files kept aside (2.5 MB)."], problems: [] });
    expect(resultLines({ removed: 1, removedBytes: 32_768, failed: ["kestrel.sqlite.foreign-2"] })).toEqual({
      done: ["Deleted 1 file kept aside (33 KB)."],
      problems: ["These files could not be deleted:", "kestrel.sqlite.foreign-2"],
    });
    expect(resultLines({ removed: 0, removedBytes: 0, failed: [] })).toEqual({ done: [], problems: [] });
  });

  it("states a small file's size in KB, never 0.0 MB", () => {
    expect(keptSize(32_768)).toBe("33 KB");
    expect(keptSize(200)).toBe("1 KB");
    expect(keptSize(0)).toBe("0 KB");
    expect(keptSize(120_000)).toBe("120 KB");
    expect(keptSize(999_499)).toBe("999 KB");
    expect(keptSize(999_500)).toBe("1.0 MB");
    expect(confirmLine({ count: 1, bytes: 32_768 })).toBe("This deletes 1 file kept aside (33 KB). It cannot be recovered.");
  });
});

describe("the missing-library notice (X4 Task 13c item 4)", () => {
  it("says the verbatim line, links the help page by its title, and can be dismissed", () => {
    const html = renderToStaticMarkup(<LibraryMissingBannerView onOpenHelp={() => {}} onDismiss={() => {}} />);
    expect(html).toContain(
      "Your library couldn&#x27;t be found, so Whimbrel started a new, empty one. If you have a backup of your library, the help page &quot;Restoring a library backup&quot; explains how to put it back.",
    );
    expect(html).toContain(">Restoring a library backup<");
    expect(html).toContain(">Dismiss<");
    expect(RESTORE_HELP_URL).toBe("https://whimbrelapp.com/help/restore-library-backup/");
  });
});
