#!/usr/bin/env python3
"""Translates the marketing site into site/pages/<locale>/, site/partials/<locale>/ and
site/i18n/layout.<locale>.html with the Anthropic API. Pages whose sourceHash still matches
their English page are skipped. Privacy, terms, compare and the changelog stay English.

  ANTHROPIC_API_KEY=... python3 website/scripts/translate-site.py --model claude-sonnet-5-5 [es fr]
"""
import argparse, hashlib, json, os, re, sys, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
PAGES = ["index.html", "pricing.html", "open-source.html", "features/meeting-notes.html", "features/screen-memory.html",
         "features/routines.html", "features/languages.html", "download/index.html", "download/mac.html",
         "download/windows.html", "download/thanks.html", "download/unavailable.html", "mobile-beta/index.html", "mobile-beta/thanks.html"]
PARTIALS = ["cta.html", "demo-video.html", "register-form.html", "app-mock.html"]
NAMES = {"es": ("neutral Latin American Spanish", "es"), "fr": ("Canadian French", "fr-CA")}
TAG = re.compile(r"</?([a-zA-Z][\w-]*)")
TOKEN = re.compile(r"\{\{\{?[^}]*\}?\}\}")
HREF = re.compile(r'\b(?:href|src|action|name|id|class|data-[\w-]+)="[^"]*"')

def front(text):
    m = re.match(r"^<!--\s*(\{[\s\S]*?\})\s*-->\s*", text)
    return (json.loads(m.group(1)), text[m.end():]) if m else (None, text)

def client(model, system, prompt):
    body = {"model": model, "max_tokens": 16000, "system": system, "messages": [{"role": "user", "content": prompt}]}
    req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=json.dumps(body).encode(), headers={
        "x-api-key": os.environ["ANTHROPIC_API_KEY"], "anthropic-version": "2023-06-01", "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        msg = json.load(r)
    if msg.get("stop_reason") == "max_tokens":
        raise RuntimeError("reply was cut off")
    return "".join(b.get("text", "") for b in msg["content"] if b["type"] == "text")

def problems(source, reply, is_page):
    errs = []
    if TAG.findall(source) != TAG.findall(reply):
        errs.append("the HTML tags changed")
    if sorted(TOKEN.findall(source)) != sorted(TOKEN.findall(reply)):
        errs.append("a {{...}} token changed")
    if sorted(HREF.findall(source)) != sorted(HREF.findall(reply)):
        errs.append("a link, class, id, name or data attribute changed")
    if is_page:
        meta, _ = front(source)
        try:
            new, _ = front(reply)
        except ValueError as e:
            return errs + [f"the JSON comment no longer parses: {e}"]
        if new is None:
            return errs + ["the JSON comment is missing"]
        if not new.get("noindex"):
            limit = 65 if meta.get("priority") == 1.0 else 54
            if len(new["title"]) > limit:
                errs.append(f"title is {len(new['title'])} characters; keep it at most {limit}")
            if not 70 <= len(new["description"]) <= 160:
                errs.append(f"description is {len(new['description'])} characters; keep it between 70 and 160")
    return errs

def translate(source, locale, model, is_page):
    name, tag = NAMES[locale]
    style = (ROOT.parent / "l10n" / "style" / f"{'fr-CA' if locale == 'fr' else 'es'}.md").read_text(encoding="utf-8")
    system = "\n".join([
        f"You translate the website of Whimbrel, a private desktop AI assistant for meetings, from US English into {name} ({tag}).",
        "It is marketing copy: write what a native copywriter would write, not a word-for-word rendering.",
        "Translate visible text, alt, aria-label, placeholder and title attributes, and the JSON comment's title, description, faq q and a values, and crumbs name values.",
        "Keep byte for byte: every HTML tag and its order, href, src, class, id, name and data-* attributes, every {{...}} and {{{...}}} token, and all other JSON keys and values.",
        "Never translate: Whimbrel, Ask, Away Mode, Today, Recap, Routines, Zoom, Ollama, Mac, Windows, macOS, or other product names. Translate the English verb \"ask\" normally.",
        "Reply with the translated file only, with no code fence.", style])
    prompt, reply = source, ""
    for _ in range(3):
        reply = client(model, system, prompt).strip() + "\n"
        errs = problems(source, reply, is_page)
        if not errs:
            return reply
        print("  retrying:", "; ".join(errs), file=sys.stderr)
        prompt = f"Your last translation had these problems: {'; '.join(errs)}.\n\nTranslate this file again:\n\n{source}"
    raise RuntimeError("; ".join(errs))

def stamp(reply, source):
    meta, body = front(reply)
    meta["sourceHash"] = hashlib.sha256(source.encode()).hexdigest()[:12]
    return f"<!--\n{json.dumps(meta, ensure_ascii=False, indent=2)}\n-->\n{body}"

def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("locales", nargs="*", default=["es", "fr"])
    p.add_argument("--model", required=True)
    a = p.parse_args()
    failed = 0
    for locale in a.locales:
        jobs = [(SITE / "pages" / f, SITE / "pages" / locale / f, True) for f in PAGES]
        jobs += [(SITE / "partials" / f, SITE / "partials" / locale / f, False) for f in PARTIALS]
        jobs.append((SITE / "layout.html", SITE / "i18n" / f"layout.{locale}.html", False))
        for src, out, is_page in jobs:
            text = src.read_text(encoding="utf-8")
            if out.exists() and (not is_page or front(out.read_text(encoding="utf-8"))[0].get("sourceHash") == hashlib.sha256(text.encode()).hexdigest()[:12]):
                continue
            print(f"{locale}: {src.relative_to(SITE)}")
            try:
                reply = translate(text, locale, a.model, is_page)
            except Exception as e:
                print(f"  FAILED: {e}", file=sys.stderr)
                failed += 1
                continue
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(stamp(reply, text) if is_page else reply, encoding="utf-8")
    return 1 if failed else 0

if __name__ == "__main__":
    sys.exit(main())
