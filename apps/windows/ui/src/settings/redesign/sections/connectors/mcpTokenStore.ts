// The one-time MCP token while its dialog is open. It lives in memory only: never in storage, never on disk, and never
// longer than the Settings tab. The store sits above the section switch (the frame draws its dialog), so a search jump
// or a shortcut to another section does not lose the token. If the whole tab goes away before the token was copied,
// the token is dropped for good and only the assistant's id and name are kept, so the way back can offer to revoke it.

import type { McpNewClient } from "../../../../generated/types";

/** A token made a moment ago, with whether it was copied. */
export type HeldToken = McpNewClient & { name: string; copied: boolean };
/** An assistant whose token was made but never copied. Holds no token. */
export interface OrphanToken {
  id: number;
  name: string;
}

let held: HeldToken | null = null;
let orphan: OrphanToken | null = null;
let version = 0;
const listeners = new Set<() => void>();
const emit = () => {
  version++;
  listeners.forEach((l) => l());
};

export const mcpToken = {
  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => void listeners.delete(listener);
  },
  /** Changes whenever anything in the store does, for useSyncExternalStore. */
  version: () => version,
  get: (): HeldToken | null => held,
  orphan: (): OrphanToken | null => orphan,
  hold(made: McpNewClient & { name: string }) {
    held = { ...made, copied: false };
    emit();
  },
  markCopied(copied: boolean) {
    if (!held || held.copied === copied) return;
    held = { ...held, copied };
    emit();
  },
  /** The dialog was closed on purpose. */
  clear() {
    held = null;
    orphan = null;
    emit();
  },
  /** The Settings tab is going away. A copied token is simply forgotten; one that was not copied leaves a notice. */
  abandon() {
    if (!held) return;
    if (!held.copied) orphan = { id: held.id, name: held.name };
    held = null;
    emit();
  },
  clearOrphan() {
    if (!orphan) return;
    orphan = null;
    emit();
  },
};
