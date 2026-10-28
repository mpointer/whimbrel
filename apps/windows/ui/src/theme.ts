// Appearance (System, Light, Dark) and the page plumbing a skin is applied through. Which skin is
// chosen, and how it reaches the page, is in skin.ts; the colours and their loader are in themes.ts.

import type { ThemeToken } from "./themes";

export type Appearance = "system" | "light" | "dark";
export type Mode = "light" | "dark";

export const APPEARANCES: readonly Appearance[] = ["system", "light", "dark"];

// Per device, so localStorage and never synced.
export const APPEARANCE_KEY = "appearance";

export const parseAppearance = (value: unknown): Appearance => (value === "light" || value === "dark" ? value : "system");

/** `textOnAccent` to `--color-text-on-accent`: the property tokens.css defines for a token. */
export const cssVariable = (token: ThemeToken): string => `--color-${token.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;

export const resolveMode = (appearance: Appearance, systemDark: boolean): Mode =>
  appearance === "system" ? (systemDark ? "dark" : "light") : appearance;

/** What the page needs from `window`; the real `window` fits, and tests pass a small fake. */
export interface ThemeWindow {
  localStorage: Pick<Storage, "getItem" | "setItem">;
  document: {
    documentElement: {
      style: { setProperty(name: string, value: string): void; removeProperty(name: string): unknown; colorScheme: string };
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
    };
  };
  matchMedia(query: string): { matches: boolean; addEventListener(type: string, listener: () => void): void; removeEventListener(type: string, listener: () => void): void };
  addEventListener(type: string, listener: (event: Event) => void): void;
  removeEventListener(type: string, listener: (event: Event) => void): void;
}
