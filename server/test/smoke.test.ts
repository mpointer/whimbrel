import { SELF } from "cloudflare:test";
import { it, expect } from "vitest";

it("returns 404 for an unrouted request", async () => {
  const response = await SELF.fetch("https://example.com/");
  expect(response.status).toBe(404);
});
