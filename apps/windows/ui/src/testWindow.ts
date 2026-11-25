// A fake `window` for the tests of how the page follows the choice (skin.test.ts).
import type { ThemeWindow } from "./theme";

/** A window that records what the page is told, with real listener bookkeeping. */
export function fakeWindow(opts: { stored?: Record<string, string>; storageThrows?: boolean; systemDark?: boolean; contrastMore?: boolean } = {}) {
  const props = new Map<string, string>();
  const attrs = new Map<string, string>();
  const data = { ...opts.stored };
  const root = {
    style: {
      colorScheme: "",
      setProperty: (name: string, value: string) => void props.set(name, value),
      removeProperty: (name: string) => props.delete(name),
    },
    setAttribute: (name: string, value: string) => void attrs.set(name, value),
    removeAttribute: (name: string) => void attrs.delete(name),
  };
  const store = {
    getItem: (key: string) => data[key] ?? null,
    setItem: (key: string, value: string) => void (data[key] = value),
  };
  let systemDark = opts.systemDark ?? false;
  let contrastMore = opts.contrastMore ?? false;
  const mediaListeners = new Set<() => void>();
  const windowListeners = new Set<(event: Event) => void>();
  const win: ThemeWindow = {
    get localStorage() {
      if (opts.storageThrows) throw new Error("storage is blocked");
      return store;
    },
    document: { documentElement: root },
    // The color scheme and the contrast ask are different queries with their own answers.
    matchMedia: (query: string) => ({
      get matches() {
        return query.includes("prefers-contrast") ? contrastMore : systemDark;
      },
      addEventListener: (_type, listener) => void mediaListeners.add(listener),
      removeEventListener: (_type, listener) => void mediaListeners.delete(listener),
    }),
    addEventListener: (_type, listener) => void windowListeners.add(listener),
    removeEventListener: (_type, listener) => void windowListeners.delete(listener),
  };
  return {
    win,
    props,
    attrs,
    root,
    data,
    listeners: () => mediaListeners.size + windowListeners.size,
    flipSystem(dark: boolean) {
      systemDark = dark;
      mediaListeners.forEach((l) => l());
    },
    flipContrast(more: boolean) {
      contrastMore = more;
      mediaListeners.forEach((l) => l());
    },
    storageEvent(key: string | null) {
      windowListeners.forEach((l) => l({ key } as unknown as Event));
    },
  };
}
