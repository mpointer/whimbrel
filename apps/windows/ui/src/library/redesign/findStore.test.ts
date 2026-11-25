import { beforeEach, describe, expect, it } from "vitest";
import { NO_FILTER } from "../../today/redesign/homeModel";
import { headerState, requestFind, requestFindWith, resetFindStore, setFindCount, setFindShown, setHeaderText, takeRequest } from "./findStore";

beforeEach(resetFindStore);

describe("the header's field and the Find page", () => {
  it("hands a request to the page once, even when the page was not there when it was made", () => {
    setHeaderText("pricing");
    requestFind("search");
    expect(takeRequest()).toMatchObject({ mode: "search" });
    expect(takeRequest()).toBeNull();
    requestFind("ask");
    expect(takeRequest()).toMatchObject({ mode: "ask" });
  });
  it("puts the words and the filters from another screen into the header", () => {
    requestFindWith("", "search", { ...NO_FILTER, app: "excel.exe" });
    expect(headerState().text).toBe("");
    expect(headerState().filter.app).toBe("excel.exe");
    expect(takeRequest()).not.toBeNull();
  });
  it("clears the count when the page leaves", () => {
    setFindShown(true);
    setFindCount("3 results");
    expect(headerState().count).toBe("3 results");
    setFindShown(false);
    expect(headerState().count).toBeNull();
  });
});

describe("what the Find page keeps across tabs", () => {
  it("holds the search it ran and its order, so coming back shows the same results", async () => {
    const { setSearched, setOrder } = await import("./findStore");
    setSearched("pricing");
    setOrder("rank");
    expect(headerState().searched).toBe("pricing");
    expect(headerState().order).toBe("rank");
    resetFindStore();
    expect(headerState().searched).toBeNull();
    expect(headerState().order).toBe("time");
  });
  it("lets the page look at a request without taking it", async () => {
    const { peekRequest } = await import("./findStore");
    requestFind("ask");
    expect(peekRequest()).toMatchObject({ mode: "ask" });
    expect(peekRequest()).toMatchObject({ mode: "ask" });
    expect(takeRequest()).toMatchObject({ mode: "ask" });
    expect(peekRequest()).toBeNull();
  });
  it("stamps when the filter was chosen, so the header and the page read the same dates", async () => {
    const { setHeaderFilter } = await import("./findStore");
    setHeaderFilter({ ...NO_FILTER, app: "excel.exe" });
    expect(headerState().filterAt).toBeGreaterThan(0);
  });
});
