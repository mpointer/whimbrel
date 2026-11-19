// A small stand-in for a DOM in tests: it expands React function components into plain nodes with working
// hooks (state, refs, effects) so a test can find a button, call its handler, and look at what the screen drew
// next. The suite has no browser DOM (every other test renders to static markup), and this is enough to test what a
// screen does when a key is pressed or a button is clicked. Not for layout: the pictures are the audit's job.

// It must not import from "react": a test replaces React's hooks with these (vi.mock), and a module the factory
// imports that itself needs React would wait on the mock forever.
type ReactNode = unknown;
const FRAGMENT = Symbol.for("react.fragment");
const ELEMENT = Symbol.for("react.element");
const isValidElement = (n: unknown): boolean => typeof n === "object" && n !== null && (n as { $$typeof?: symbol }).$$typeof === ELEMENT;

export interface TNode {
  tag: string;
  props: Record<string, unknown>;
  children: (TNode | string)[];
  parent?: TNode;
}
type Child = TNode | string;

interface Ctx {
  tree: Tree;
  path: string;
  idx: number;
}
let current: Ctx | null = null;
const here = () => {
  if (!current) throw new Error("a hook was called outside a Tree render");
  return current;
};

/** The hooks the tree implements, to put in place of React's with `vi.mock("react", ...)`. */
export const stubHooks = {
  useState<T>(init: T | (() => T)) {
    const c = here();
    const slots = c.tree.slots(c.path);
    const i = c.idx++;
    if (!(i in slots)) slots[i] = { value: typeof init === "function" ? (init as () => T)() : init };
    const slot = slots[i] as { value: T };
    const tree = c.tree;
    const set = (v: T | ((prev: T) => T)) => {
      const next = typeof v === "function" ? (v as (prev: T) => T)(slot.value) : v;
      if (!Object.is(next, slot.value)) {
        slot.value = next;
        tree.dirty = true;
      }
    };
    return [slot.value, set] as const;
  },
  useReducer<S, A>(reducer: (s: S, a: A) => S, init: S) {
    const c = here();
    const slots = c.tree.slots(c.path);
    const i = c.idx++;
    if (!(i in slots)) slots[i] = { value: init };
    const slot = slots[i] as { value: S };
    const tree = c.tree;
    const dispatch = (a: A) => {
      const next = reducer(slot.value, a);
      if (!Object.is(next, slot.value)) {
        slot.value = next;
        tree.dirty = true;
      }
    };
    return [slot.value, dispatch] as const;
  },
  useRef<T>(init: T) {
    const c = here();
    const slots = c.tree.slots(c.path);
    const i = c.idx++;
    if (!(i in slots)) slots[i] = { value: { current: init } };
    return (slots[i] as { value: { current: T } }).value;
  },
  useId() {
    return `id${here().path.replace(/\W/g, "")}`;
  },
  useEffect(fn: () => void | (() => void), deps?: unknown[]) {
    const c = here();
    const slots = c.tree.slots(c.path);
    const i = c.idx++;
    const prev = slots[i] as { deps?: unknown[] } | undefined;
    const same = prev && deps && prev.deps && deps.length === prev.deps.length && deps.every((d, k) => Object.is(d, prev.deps![k]));
    slots[i] = { deps };
    if (!same) c.tree.effects.push(fn);
  },
  useSyncExternalStore<T>(_subscribe: unknown, get: () => T) {
    here().idx++;
    return get();
  },
  useMemo<T>(fn: () => T, deps?: unknown[]) {
    const c = here();
    const slots = c.tree.slots(c.path);
    const i = c.idx++;
    const prev = slots[i] as { value: T; deps?: unknown[] } | undefined;
    if (prev && deps && prev.deps && deps.length === prev.deps.length && deps.every((d, k) => Object.is(d, prev.deps![k]))) return prev.value;
    const value = fn();
    slots[i] = { value, deps };
    return value;
  },
  useCallback<T>(fn: T) {
    here().idx++;
    return fn;
  },
};

/** React with its hooks replaced by `stubHooks`; the argument of a `vi.mock("react", ...)` factory. */
export function withStubbedHooks<T extends object>(actual: T): T {
  const hooks = { ...stubHooks, useLayoutEffect: stubHooks.useEffect };
  return { ...actual, ...hooks, default: { ...((actual as { default?: object }).default ?? actual), ...hooks } };
}

export class Tree {
  private store = new Map<string, Record<number, unknown>>();
  effects: (() => void | (() => void))[] = [];
  dirty = false;
  root: Child[] = [];

  constructor(private make: () => unknown) {}

  slots(path: string) {
    let s = this.store.get(path);
    if (!s) this.store.set(path, (s = {}));
    return s;
  }

  /** Draws, runs the effects that are due, and draws again while state keeps changing. */
  render(): this {
    for (let pass = 0; pass < 12; pass++) {
      this.dirty = false;
      this.effects = [];
      this.root = this.expand(this.make(), "r");
      for (const fx of this.effects) fx();
      if (!this.dirty) return this;
    }
    throw new Error("the screen kept changing state without settling");
  }

  /** After an async answer (a mocked API call) has had time to land. */
  async settle(): Promise<this> {
    for (let i = 0; i < 6; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    return this.render();
  }

  private expand(node: ReactNode, path: string): Child[] {
    if (node === null || node === undefined || typeof node === "boolean") return [];
    if (typeof node === "string" || typeof node === "number") return [String(node)];
    if (Array.isArray(node)) return node.flatMap((n, i) => this.expand(n, `${path}.${i}`));
    if (!isValidElement(node)) return [];
    const el = node as { type: unknown; props: Record<string, unknown>; key: string | null };
    const props = el.props;
    const kids = (path: string) => this.expand(props.children as ReactNode, path);
    if (el.type === FRAGMENT) return kids(`${path}f`);
    if (typeof el.type === "function") {
      const saved = current;
      current = { tree: this, path: `${path}${el.key ? `:${el.key}` : ""}`, idx: 0 };
      try {
        return this.expand((el.type as (p: unknown) => ReactNode)(props), `${current.path}>`);
      } finally {
        current = saved;
      }
    }
    if (typeof el.type !== "string") return [];
    const host: TNode = { tag: el.type, props, children: kids(`${path}${el.key ? `:${el.key}` : ""}/${el.type}`) };
    for (const c of host.children) if (typeof c !== "string") c.parent = host;
    return [host];
  }

  all(test: (n: TNode) => boolean): TNode[] {
    const out: TNode[] = [];
    const walk = (c: Child) => {
      if (typeof c === "string") return;
      if (test(c)) out.push(c);
      c.children.forEach(walk);
    };
    this.root.forEach(walk);
    return out;
  }
  one(test: (n: TNode) => boolean, what = "node"): TNode {
    const found = this.all(test);
    if (found.length !== 1) throw new Error(`expected one ${what}, found ${found.length}`);
    return found[0];
  }
  /** The button whose words are exactly `label`. */
  button(label: string): TNode {
    return this.one((n) => n.tag === "button" && text(n).trim() === label, `button "${label}"`);
  }
  hasButton(label: string): boolean {
    return this.all((n) => n.tag === "button" && text(n).trim() === label).length > 0;
  }
  /** The text field (input or text area) the visible label `label` names. */
  field(label: string): TNode {
    const lab = this.one((n) => n.tag === "label" && n.props.htmlFor !== undefined && text(n).trim() === label, `label "${label}"`);
    return this.one((n) => (n.tag === "input" || n.tag === "textarea") && n.props.id === lab.props.htmlFor, `field "${label}"`);
  }
  text(): string {
    return this.root.map((c) => (typeof c === "string" ? c : text(c))).join(" ");
  }
  has(words: string): boolean {
    return this.text().includes(words);
  }
}

export function text(n: Child): string {
  return typeof n === "string" ? n : n.children.map(text).join("");
}

/** Types into a field: calls its change handler as the browser would. */
export function type(field: TNode, value: string) {
  (field.props.onChange as (e: unknown) => void)({ target: { value } });
}

/** A key press for a `keydown` handler, going to an element of the given tag. */
export function press(node: TNode, key: string, target: { tag?: string; type?: string; role?: string; inDialog?: boolean; ctrl?: boolean } = {}) {
  let prevented = false;
  let stopped = false;
  const el = {
    tagName: (target.tag ?? "div").toUpperCase(),
    type: target.type,
    getAttribute: (name: string) => (name === "role" ? (target.role ?? null) : null),
    closest: () => (target.inDialog ? {} : null),
  };
  (node.props.onKeyDown as (e: unknown) => void)({
    key,
    ctrlKey: !!target.ctrl,
    metaKey: false,
    shiftKey: false,
    defaultPrevented: false,
    target: el,
    nativeEvent: { isComposing: false },
    preventDefault: () => (prevented = true),
    stopPropagation: () => (stopped = true),
  });
  return { prevented, stopped };
}
