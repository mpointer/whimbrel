// The route preview (`chat_route_preview`) for the Ask buttons. Each call loads keys, probes Ollama and reads history, so the page asks as
// little as it can: one request in flight per (words, conversation, filter), the last result kept for a few seconds, and nothing kept
// across a change of settings. The call that actually sends a question reads fresh and never trusts what was kept.

import { useEffect, useRef, useState } from "react";
import { chatRoutePreview } from "../../api";
import type { FindFilterInput, RoutePreviewView } from "../../generated/types";
import { filterKey } from "./findModel";

/** How long a result is trusted. The button is a promise, so this is short, and `clear` ends it at once on a settings change. */
export const PREVIEW_TTL_MS = 5000;

export interface PreviewCache {
  /** The preview for these words. `fresh` skips what is kept and what is on its way, and reads again. Rejects when the read fails. */
  read(text: string, conversationId: number | null, filter: FindFilterInput, options?: { fresh?: boolean }): Promise<RoutePreviewView | null>;
  /** Forget everything, and drop any read still on its way. Whoever subscribed is told, so a label that showed a kept preview reads again. */
  clear(): void;
  /** Called after each `clear`. Returns the way to stop. */
  subscribe(listener: () => void): () => void;
  /** Called with each preview read (a fresh one too) and its `previewKey`, so a label shows what a press was sent as. Returns the way to stop. */
  watch(listener: (key: string, view: RoutePreviewView | null) => void): () => void;
}

export function createPreviewCache(
  fetch: (text: string, conversationId: number | null, filter: FindFilterInput) => Promise<RoutePreviewView | null>,
  now: () => number,
  ttlMs: number = PREVIEW_TTL_MS,
): PreviewCache {
  const kept = new Map<string, { at: number; view: RoutePreviewView | null }>();
  const onItsWay = new Map<string, Promise<RoutePreviewView | null>>();
  let generation = 0;
  const listeners = new Set<() => void>();
  const watchers = new Set<(key: string, view: RoutePreviewView | null) => void>();
  const keyOf = (text: string, conversationId: number | null, filter: FindFilterInput) => JSON.stringify([text, conversationId, filterKey(filter)]);

  const start = (key: string, text: string, conversationId: number | null, filter: FindFilterInput, track: boolean): Promise<RoutePreviewView | null> => {
    const mine = generation;
    const request = fetch(text, conversationId, { ...filter }).then((view) => {
      if (mine === generation) {
        kept.set(key, { at: now(), view });
        watchers.forEach((w) => w(key, view));
      }
      return view;
    });
    if (!track) return request;
    onItsWay.set(key, request);
    const forget = () => {
      if (onItsWay.get(key) === request) onItsWay.delete(key);
    };
    request.then(forget, forget);
    return request;
  };

  return {
    read(text, conversationId, filter, options) {
      const key = keyOf(text, conversationId, filter);
      if (options?.fresh) return start(key, text, conversationId, filter, false);
      const hit = kept.get(key);
      if (hit && now() - hit.at < ttlMs) return Promise.resolve(hit.view);
      return onItsWay.get(key) ?? start(key, text, conversationId, filter, true);
    },
    clear() {
      generation += 1;
      kept.clear();
      onItsWay.clear();
      listeners.forEach((l) => l());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    watch(listener) {
      watchers.add(listener);
      return () => void watchers.delete(listener);
    },
  };
}

/** The page's one cache. */
export const routePreviews = createPreviewCache(chatRoutePreview, Date.now);

/** What a preview was read for. A label shows a preview's promise only while this is still what is being asked. */
export const previewKey = (text: string, conversationId: number | null, filter: FindFilterInput): string => JSON.stringify([text.trim(), conversationId, filterKey(filter)]);

/** The preview, if it was read for `key`; otherwise null (a label then says plain "Ask"). */
export const previewFor = (entry: { key: string; view: RoutePreviewView | null } | null, key: string): RoutePreviewView | null => (entry && entry.key === key ? entry.view : null);

/** The preview for `text`, read a moment after it stops changing, and shown only while it matches the words, the filters and the conversation
 *  now. Null with no words, no model to ask, no answer yet, or a preview that was read for something else. Read again after a clear. */
export function useRoutePreview(text: string, conversationId: number | null, filter: FindFilterInput, enabled: boolean): RoutePreviewView | null {
  const [entry, setEntry] = useState<{ key: string; view: RoutePreviewView | null } | null>(null);
  const [cleared, setCleared] = useState(0);
  // The settings changed: what was read no longer holds, so the label says plain "Ask" until the new read arrives.
  useEffect(
    () =>
      routePreviews.subscribe(() => {
        setEntry(null);
        setCleared((n) => n + 1);
      }),
    [],
  );
  const filterK = filterKey(filter);
  const latest = useRef(filter);
  latest.current = filter;
  const key = previewKey(text, conversationId, filter);
  // A press reads the route fresh just before it sends: the label takes that read, so it never says plain "Ask" for a question that
  // went as one for this PC (or the other way round).
  const keyNow = useRef(key);
  keyNow.current = key;
  useEffect(() => routePreviews.watch((k, view) => k === keyNow.current && setEntry({ key: k, view })), []);
  useEffect(() => {
    const words = text.trim();
    if (!words || !enabled) {
      setEntry(null);
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      routePreviews.read(words, conversationId, latest.current).then(
        (p) => live && setEntry({ key, view: p }),
        () => live && setEntry({ key, view: null }),
      );
    }, 350);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [text, conversationId, filterK, enabled, cleared]); // eslint-disable-line react-hooks/exhaustive-deps
  return previewFor(entry, key);
}
