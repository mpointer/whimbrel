import { describe, expect, it } from "vitest";
import { applyEvent, newTurn } from "../chat/chatModel";
import { budgetNoticeText, monthName } from "./BudgetAlertView";

const notice = { provider: "openai", spentCents: 2000, budgetCents: 2500, month: "2026-11" };

describe("budget notices", () => {
  it("say 80% with the amounts, and 100% with the month the provider waits for", () => {
    expect(budgetNoticeText({ ...notice, threshold: 80 }, "en")).toBe("OpenAI is at 80% of this month's budget ($20.00 of $25.00).");
    expect(budgetNoticeText({ ...notice, threshold: 100, spentCents: 2500 }, "en")).toBe(
      "OpenAI reached this month's budget. Whimbrel will use your other providers until November ends.",
    );
  });

  it("name the UTC month in the UI language", () => {
    expect(monthName("2026-12", "en")).toBe("December");
    expect(monthName("2026-01", "de")).toBe("Januar");
    expect(monthName("garbage", "en")).toBe("garbage");
  });

  it("mark an Ask turn that failed on a spent budget, so it offers the settings", () => {
    const spent = applyEvent(newTurn("q"), {
      kind: "failed",
      partial: false,
      error: { kind: "budget_spent", provider: "anthropic", spent_cents: 2500, budget_cents: 2500 },
    });
    expect(spent.budgetSpent).toBe(true);
    expect(spent.error).toBe("Your Anthropic budget for this month is spent ($25.00 of $25.00). Raise it in Settings → AI.");
    const other = applyEvent(newTurn("q"), { kind: "failed", partial: false, error: { kind: "provider_failure" } });
    expect(other.budgetSpent).toBe(false);
  });
});
