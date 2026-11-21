import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { PlusAiPromptAnswer } from "../generated/types";
import { ROUTING_CHANGED_EVENT } from "./aiModel";
import type { PlusAiPromptView } from "../generated/types";
import { afterAnswer, nextAfter, NOT_PLUS_MS, PlusAiPromptDialog, SHOWING_MS, startChecking, WAIT_MS, wasTyping, type CheckHost } from "./PlusAiPrompt";

const NOTE = "Core's data note, as core words it.";

type Props = { onClick?: () => void; onCancel?: () => void; children?: ReactNode; details?: ReactNode; "data-initial"?: string };

/** Every element in the tree that `match` accepts, in order (looking into children and `details`). */
function find(node: ReactNode, match: (el: ReactElement<Props>) => boolean): ReactElement<Props>[] {
  if (Array.isArray(node)) return node.flatMap((n) => find(n, match));
  if (!isValidElement(node)) return [];
  const el = node as ReactElement<Props>;
  const here = match(el) ? [el] : [];
  return [...here, ...find(el.props.children, match), ...find(el.props.details, match)];
}

describe("the Plus Use Whimbrel AI question", () => {
  const render = (onAnswer: (choice: PlusAiPromptAnswer) => void = () => {}, focusDialog = false) =>
    PlusAiPromptDialog({ note: NOTE, busy: false, error: null, focusDialog, onAnswer });

  it("asks, shows core's note as given and where to change it, with three answers", () => {
    const html = renderToStaticMarkup(render());
    const note = NOTE.replace(/'/g, "&#x27;");
    expect(html).toContain("Plus includes a monthly Whimbrel AI allowance. Use it for your AI features?");
    expect(html).toContain(note);
    expect(html).toContain("Settings → AI");
    expect(html).toContain("Exclude from hosted models");
    expect(html).not.toContain("on your computer");
    for (const label of ["Use Whimbrel AI", "Keep my current setup", "Not now"]) expect(html).toContain(label);
    // The note is the dialog's description.
    const described = /aria-describedby="([^"]+)"/.exec(html)![1];
    expect(html).toContain(`id="${described}">${note}`);
    // The design system's dialog, on its own layer (the reviewed version's scrim covered its card).
    expect(html).toContain('class="w-dialog-scrim"');
    expect(html).toContain('role="alertdialog"');
  });

  it("answers with each button, Use Whimbrel AI is the default, Esc is Not now", () => {
    const onAnswer = vi.fn();
    const tree = render(onAnswer);
    const buttons = find(tree, (el) => typeof el.props.onClick === "function");
    const byLabel = (label: string) => buttons.find((b) => b.props.children === label)!;
    byLabel("Use Whimbrel AI").props.onClick!();
    byLabel("Keep my current setup").props.onClick!();
    byLabel("Not now").props.onClick!();
    expect(byLabel("Use Whimbrel AI").props["data-initial"]).toBe("");
    find(tree, (el) => typeof el.props.onCancel === "function")[0].props.onCancel!();
    expect(onAnswer.mock.calls.map((c) => c[0])).toEqual(["use_whimbrel", "keep", "not_now", "not_now"]);
  });

  it("checks again, waits for focus, stops, and never stacks over another dialog", () => {
    expect(nextAfter("show", false)).toEqual({ kind: "show" });
    expect(nextAfter("show", true)).toEqual({ kind: "again", ms: SHOWING_MS });
    expect(nextAfter("wait", false)).toEqual({ kind: "again", ms: WAIT_MS });
    // Not Plus by this session's read: rarely, and on focus, so a plan bought meanwhile is still asked about.
    expect(nextAfter("not_plus", false)).toEqual({ kind: "again", ms: NOT_PLUS_MS, onFocus: true });
    expect(nextAfter("done", false)).toEqual({ kind: "stop" });
  });

  it("keeps its buttons focusable while an answer saves, and ignores presses then", () => {
    const onAnswer = vi.fn();
    const tree = PlusAiPromptDialog({ note: NOTE, busy: true, error: null, focusDialog: false, onAnswer });
    const html = renderToStaticMarkup(tree);
    expect(html).not.toContain("disabled=\"\"");
    expect(html.match(/aria-disabled="true"/g)?.length).toBe(3);
    for (const b of find(tree, (el) => typeof el.props.onClick === "function")) b.props.onClick!();
    expect(onAnswer).not.toHaveBeenCalled();
  });

  it("tells an open Settings → AI form to read routing again only after an applied Use Whimbrel AI", () => {
    const target = new EventTarget();
    const heard = vi.fn();
    target.addEventListener(ROUTING_CHANGED_EVENT, heard);
    afterAnswer("keep", true, target);
    afterAnswer("not_now", true, target);
    afterAnswer("use_whimbrel", false, target);
    expect(heard).not.toHaveBeenCalled();
    afterAnswer("use_whimbrel", true, target);
    expect(heard).toHaveBeenCalledTimes(1);
  });

  it("doesn't take the default when the user was typing", () => {
    expect(wasTyping({ tagName: "INPUT", type: "text" })).toBe(true);
    expect(wasTyping({ tagName: "TEXTAREA" })).toBe(true);
    expect(wasTyping({ tagName: "DIV", isContentEditable: true })).toBe(true);
    expect(wasTyping({ tagName: "INPUT", type: "checkbox" })).toBe(false);
    expect(wasTyping({ tagName: "BUTTON" })).toBe(false);
    expect(wasTyping(null)).toBe(false);
  });
});

describe("the check loop", () => {
  /** A host on fake timers, with a fake focus event; `answers` are the checks' results in turn. */
  function host(answers: PlusAiPromptView["state"][], over: Partial<CheckHost> = {}) {
    const focus = new Set<() => void>();
    let i = 0;
    const calls = { checks: 0, shown: 0, closed: 0 };
    const h: CheckHost = {
      check: () => {
        calls.checks++;
        const state = answers[Math.min(i++, answers.length - 1)];
        return Promise.resolve({ state, note: state === "show" ? "note" : null });
      },
      showing: false,
      busy: () => false,
      otherDialogOpen: () => false,
      show: () => calls.shown++,
      close: () => calls.closed++,
      setTimer: (f, ms) => setTimeout(f, ms),
      clearTimer: (t) => clearTimeout(t),
      onFocus: (f) => focus.add(f),
      offFocus: (f) => focus.delete(f),
      ...over,
    };
    const fireFocus = () => [...focus].forEach((f) => f());
    return { h, calls, focus, fireFocus };
  }
  const flush = () => vi.advanceTimersByTimeAsync(0);

  it("keeps one chain when the window regains focus while not Plus, and leaves nothing behind", async () => {
    vi.useFakeTimers();
    try {
      const { h, calls, focus, fireFocus } = host(["not_plus"]);
      const stop = startChecking(h);
      await flush();
      for (let k = 0; k < 5; k++) {
        fireFocus();
        await flush();
      }
      expect(calls.checks).toBe(6);
      expect(vi.getTimerCount()).toBe(1);
      expect(focus.size).toBe(1);
      await vi.advanceTimersByTimeAsync(NOT_PLUS_MS);
      expect(calls.checks).toBe(7);
      stop();
      expect(vi.getTimerCount()).toBe(0);
      expect(focus.size).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits before a plan read, then shows once a check says so", async () => {
    vi.useFakeTimers();
    try {
      const { h, calls } = host(["wait", "show"]);
      const stop = startChecking(h);
      await flush();
      expect(calls.shown).toBe(0);
      await vi.advanceTimersByTimeAsync(WAIT_MS);
      expect(calls.shown).toBe(1);
      stop();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("closes a dialog that no longer applies, but not while an answer saves", async () => {
    vi.useFakeTimers();
    try {
      let busy = true;
      const { h, calls } = host(["wait"], { showing: true, busy: () => busy });
      const stop = startChecking(h);
      await vi.advanceTimersByTimeAsync(SHOWING_MS);
      expect(calls.closed).toBe(0);
      busy = false;
      await vi.advanceTimersByTimeAsync(SHOWING_MS);
      expect(calls.closed).toBe(1);
      stop();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
