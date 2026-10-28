import { it, expect } from "vitest";
import { isEmailAllowed } from "../src/allowlist";

it("denies every email when the allowlist is undefined", () => {
  expect(isEmailAllowed("user@example.com", undefined)).toBe(false);
});

it("denies every email when the allowlist is an empty string", () => {
  expect(isEmailAllowed("user@example.com", "")).toBe(false);
});

it("denies every email when the allowlist is only whitespace", () => {
  expect(isEmailAllowed("user@example.com", "   ")).toBe(false);
});

it("allows an email that exactly matches a single-entry allowlist", () => {
  expect(isEmailAllowed("owner@example.com", "owner@example.com")).toBe(true);
});

it("denies an email not present in the allowlist", () => {
  expect(isEmailAllowed("someone-else@example.com", "owner@example.com")).toBe(false);
});

it("matches case-insensitively", () => {
  expect(isEmailAllowed("Owner@Example.com", "owner@example.com")).toBe(true);
});

it("tolerates whitespace around comma-separated entries", () => {
  expect(isEmailAllowed("b@example.com", "a@example.com,  b@example.com ,c@example.com")).toBe(true);
});

it("matches one entry among several", () => {
  const list = "a@example.com,b@example.com,c@example.com";
  expect(isEmailAllowed("a@example.com", list)).toBe(true);
  expect(isEmailAllowed("c@example.com", list)).toBe(true);
  expect(isEmailAllowed("d@example.com", list)).toBe(false);
});
