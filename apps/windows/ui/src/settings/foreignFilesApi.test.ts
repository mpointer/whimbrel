import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args), Channel: class {} }));

import { foreignFilesDelete, foreignFilesList, launchDismissLibraryMissing, launchLibraryMissing } from "../api";

describe("files kept aside and the missing-library banner: the shell calls", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });

  it("lists without deleting, and deletes only with the count the confirmation named", async () => {
    await foreignFilesList();
    expect(invoke).toHaveBeenLastCalledWith("foreign_files_list");
    expect(invoke).not.toHaveBeenCalledWith("foreign_files_delete", expect.anything());
    await foreignFilesDelete(3);
    expect(invoke).toHaveBeenLastCalledWith("foreign_files_delete", { confirmed: 3 });
  });

  it("asks whether to show the banner, and Dismiss tells the shell", async () => {
    await launchLibraryMissing();
    expect(invoke).toHaveBeenLastCalledWith("launch_library_missing");
    await launchDismissLibraryMissing();
    expect(invoke).toHaveBeenLastCalledWith("launch_dismiss_library_missing");
  });
});
