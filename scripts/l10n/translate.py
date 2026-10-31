#!/usr/bin/env python3
"""Whimbrel's string catalogs: extract, check, AI-translate, review and apply.

    translate.py extract [--apple <xcloc> <catalog>]
    translate.py check [--apple <xcloc> <catalog>]      no network, no key
    translate.py translate <lang> --model M [--catalog PATH]
    translate.py review <lang> --sample N [--out CSV]
    translate.py apply <csv> [--lang L]
    translate.py pseudo                                  (Task 14)

Stdlib only; run with python3 -I. Machine translations are written with state "needs_review";
`apply` writes reviewed rows as "translated".
"""
import argparse
import csv
import json
import os
import random
import re
import sys
import unicodedata
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
L10N = ROOT / "l10n"
WINDOWS_CATALOG = "apps/windows/ui/src/l10n/Localizable.xcstrings"
WINDOWS_SOURCES = [("apps/windows/ui/src", "t", (".ts", ".tsx")), ("apps/windows/src-tauri/src", "tr", (".rs",))]
CATALOGS = [
    "apps/macos/Whimbrel/Localizable.xcstrings",
    "apps/macos/Whimbrel/InfoPlist.xcstrings",
    "apps/mobile/ios/Whimbrel/Localizable.xcstrings",
    "apps/mobile/ios/Whimbrel/InfoPlist.xcstrings",
    WINDOWS_CATALOG,
    "core/memory/l10n/Localizable.xcstrings",
]  # plus every skins/*.json with a "voice" map, see catalog_paths()
BATCH = 50

# Copied from core/skins/src/format.rs VoiceKey::limit (test_voice_limits_match_rust keeps them in step).
VOICE_LIMITS = {
    "greetingMorning": 48, "greetingAfternoon": 48, "greetingEvening": 48,
    "askPlaceholder": 40,
    "upNextHeader": 24, "upcomingHeader": 24, "earlierHeader": 24, "recapHeader": 24,
    "emptyDay": 80, "emptyEarlier": 80,
    "tagline": 60,
}
MEASURED_NAME_LENGTH = 30

PLACEHOLDER = re.compile(r"%(?:\d+\$)?[-+ #0]*\d*(?:\.\d+)?(?:hh|h|ll|l|q|z|t|j|L)?[@dDiuUxXoOfFeEgGcCsSpaA]|\{\w+\}")
MARKUP = [re.compile(p, re.M) for p in (r"<[A-Za-z/][^>]*>", r"\*\*", r"`", r"\]\(", r"^#+\s", r"__")]


# ---------------------------------------------------------------- catalogs

def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path, data, like=None):
    """Write in the file's existing style. An .xcstrings gets Xcode's style (`" : "`, a blank line in
    empty objects) unless it is already plain multi-line JSON (the hand-written core catalog)."""
    path = Path(path)
    old = like if like is not None else (path.read_text(encoding="utf-8") if path.exists() else "")
    xcode = path.suffix == ".xcstrings" and '": ' not in old
    text = json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True,
                      separators=(",", " : ") if xcode else (",", ": "))
    if xcode:
        text = re.sub(r"^( *)(.*)\{\}(,?)$", lambda m: f"{m[1]}{m[2]}{{\n\n{m[1]}}}{m[3]}", text, flags=re.M)
    if old.endswith("\n") or not xcode:
        text += "\n"
    path.write_text(text, encoding="utf-8")


def catalog_paths(root=ROOT):
    paths = [root / p for p in CATALOGS if (root / p).exists()]
    paths += [p for p in sorted((root / "skins").glob("*.json")) if "voice" in read_json(p)]
    return paths


def letters(s):
    return re.search(r"[^\W\d_]", PLACEHOLDER.sub("", s)) is not None


class Catalog:
    """An .xcstrings catalog, or a skin manifest (`voice` is the source, `voiceLocalized` the translations)."""

    def __init__(self, path, data, text):
        self.path, self.data, self.text = Path(path), data, text
        self.skin = self.path.suffix == ".json"
        self.dirty = False

    @classmethod
    def load(cls, path):
        text = Path(path).read_text(encoding="utf-8")
        return cls(path, json.loads(text), text)

    def entries(self):
        out = []
        if self.skin:
            for key, source in self.data.get("voice", {}).items():
                limit = VOICE_LIMITS.get(key)
                out.append({"catalog": self, "key": key, "source": source, "plural": False, "limit": limit,
                            "comment": f"Skin voice line ({key}) for the {self.data.get('name', '')} skin; "
                                       f"at most {limit} characters with {{name}} as a {MEASURED_NAME_LENGTH}-character name."})
            return out
        for key, e in self.data.get("strings", {}).items():
            if e.get("shouldTranslate") is False:
                continue
            en = e.get("localizations", {}).get("en", {})
            if "variations" in en:
                source = {c: v["stringUnit"]["value"] for c, v in en["variations"].get("plural", {}).items()}
            else:
                source = en.get("stringUnit", {}).get("value") or key
            out.append({"catalog": self, "key": key, "source": source, "comment": e.get("comment", ""),
                        "plural": isinstance(source, dict) or "{count}" in key, "limit": None})
        return out

    def langs(self, key):
        if self.skin:
            return [l for l, m in self.data.get("voiceLocalized", {}).items() if key in m]
        return list(self.data["strings"].get(key, {}).get("localizations", {}))

    def get(self, lang, key):
        """(value, state): value is a string, a {category: string} dict, or None."""
        if self.skin:
            return self.data.get("voiceLocalized", {}).get(lang, {}).get(key), None
        loc = self.data["strings"].get(key, {}).get("localizations", {}).get(lang)
        if not loc:
            return None, None
        if "variations" in loc:
            cases = loc["variations"].get("plural", {})
            states = {v["stringUnit"].get("state") for v in cases.values()}
            return {c: v["stringUnit"]["value"] for c, v in cases.items()}, ("needs_review" if "needs_review" in states else states.pop() if states else None)
        u = loc.get("stringUnit", {})
        return u.get("value"), u.get("state")

    def set(self, lang, key, value, state):
        self.dirty = True
        if self.skin:
            self.data.setdefault("voiceLocalized", {}).setdefault(lang, {})[key] = value
            return
        locs = self.data["strings"].setdefault(key, {}).setdefault("localizations", {})
        if isinstance(value, dict):
            locs[lang] = {"variations": {"plural": {c: {"stringUnit": {"state": state, "value": v}} for c, v in value.items()}}}
        else:
            locs[lang] = {"stringUnit": {"state": state, "value": value}}

    def save(self):
        if self.dirty:
            write_json(self.path, self.data, like=self.text)
            self.dirty = False


def load_all(paths=None):
    return [Catalog.load(p) for p in (paths or catalog_paths())]


# ---------------------------------------------------------------- validation

def base_language(lang):
    return re.split(r"[-_]", lang)[0]


def plural_category(lang, n):
    """Mirrors core/memory/src/l10n.rs plural_category: French and Portuguese put 0 in "one"."""
    if base_language(lang) in ("fr", "pt"):
        return "one" if n in (0, 1) else "other"
    return "one" if n == 1 else "other"


def required_plural_categories(lang):
    return {plural_category(lang, n) for n in range(0, 3)}


SPEC = re.compile(r"%(?:(\d+)\$)?[-+ #0']*\d*(?:\.\d+)?(hh|h|ll|l|q|z|t|j|L)?([@dDiuUxXoOfFeEgGaAcCsSp])")
INT, FLOAT = set("dDiuUxXoO"), set("fFeEgGaA")


def placeholders(s):
    """({slot: type}, errors). A slot is a printf argument index (sequential when unnumbered, N for
    %N$) or a {name}; the type is the length modifier plus the conversion class, so a translation
    that swaps %lld and %@ (an integer read as an object at runtime) no longer matches."""
    slots, errors, nxt, kinds = {}, [], 0, set()
    i = 0
    while True:
        i = s.find("%", i)
        if i < 0:
            break
        if s.startswith("%%", i):
            i += 2
            continue
        m = SPEC.match(s, i)
        if not m:
            errors.append(f"unrecognized format sequence {s[i:i + 4]!r}")
            i += 1
            continue
        pos, length, conv = m.groups()
        kinds.add("numbered" if pos else "unnumbered")
        if pos:
            slot = int(pos)
        else:
            nxt += 1
            slot = nxt
        kind = "int" if conv in INT else "float" if conv in FLOAT else conv
        typ = (length or "") + kind if kind != "float" else "float"
        if slots.get(slot, typ) != typ:
            errors.append(f"argument {slot} is used as both {slots[slot]} and {typ}")
        slots[slot] = typ
        i = m.end()
    if len(kinds) > 1:
        errors.append("mixes numbered (%1$@) and unnumbered (%@) specifiers")
    for name in re.findall(r"\{(\w+)\}", s):
        slots[name] = "{}"
    return slots, errors


def check_placeholders(source, value, subset=False):
    want, source_errors = placeholders(source)
    if source_errors:
        # Not a printf string (core's chrono pattern "%A, %B %-d"): fields may be reordered, not changed.
        fields = lambda t: sorted(re.findall(r"%[-_0^#]?[A-Za-z%]|\{\w+\}", t))
        return [] if fields(source) == fields(value) else [f"fields {fields(value)} do not match the English {fields(source)}"]
    got, errors = placeholders(value)
    if errors:
        return errors
    if subset:
        extra = {k: v for k, v in got.items() if want.get(k) != v}
        return [f"placeholders {extra} are not in the English {want}"] if extra else []
    return [] if want == got else [f"placeholders {got} do not match the English {want}"]


def check_plural_shape(entry, value, lang):
    if not entry["plural"]:
        return [] if isinstance(value, str) else ["expected a string, not plural forms"]
    if not isinstance(value, dict):
        return [f"expected plural forms {sorted(required_plural_categories(lang))}"]
    missing = required_plural_categories(lang) - set(value)
    return [f"missing plural form(s) {sorted(missing)} for {lang}"] if missing else []


def source_form(entry, category):
    s = entry["source"]
    return s if isinstance(s, str) else s.get(category, s.get("other", ""))


def forms(value):
    return value.items() if isinstance(value, dict) else [(None, value)]


def length_limit(entry, source):
    if entry.get("limit"):
        return entry["limit"]
    n = len(source)
    return max(n * 3 // 2, n + 10)


def measured(s):
    # ponytail: code points after NFC, not grapheme clusters (stdlib has none); exact for the Latin scripts we ship
    return len(unicodedata.normalize("NFC", s.replace("{name}", "x" * MEASURED_NAME_LENGTH)))


def validate(entry, value, lang, glossary, budget=True):
    """Every rule a machine translation must pass. Returns a list of error strings.
    budget=False (a reviewer's own text) skips the length budget."""
    errors = check_plural_shape(entry, value, lang)
    if errors:
        return errors
    for cat, v in forms(value):
        src = source_form(entry, cat)
        if not isinstance(v, str) or not v.strip():
            errors.append(f"{cat or 'value'} is empty")
            continue
        # A "one" form may spell the number out ("une réunion") and drop the count.
        errors += check_placeholders(src, v, subset=cat == "one")
        for term in glossary.get("do_not_translate", []):
            not_when = glossary.get("not_when", {}).get(term)
            if re.search(rf"\b{re.escape(term)}\b", src) and term not in v and not (not_when and re.search(not_when, src)):
                errors.append(f'"{term}" must stay in English, verbatim')
        for term, by_lang in glossary.get("fixed", {}).items():
            fixed = by_lang.get(lang) or by_lang.get(base_language(lang))
            if fixed and re.search(rf"\b{re.escape(term)}\b", src) and fixed not in v:
                errors.append(f'"{term}" must be translated as "{fixed}"')
        limit = length_limit(entry, src)
        if budget and (measured(v) if entry.get("limit") else len(v)) > limit:
            errors.append(f"too long: {len(v)} characters, the budget is {limit}")
        for pat in MARKUP:
            if len(pat.findall(v)) > len(pat.findall(src)):
                errors.append(f"adds markdown or HTML ({pat.pattern}) that the English does not have")
    return errors


def check(catalogs, languages, jsx_dir=None):
    """Offline checks over the catalogs. Returns a list of error strings."""
    errors = []
    exempt = set(languages.get("variants", [])) | set(languages.get("debug", []))
    required = [l for l in languages.get("shipped", []) if l not in exempt]
    for c in catalogs:
        for e in c.entries():
            where = f"{c.path.name}: {e['key']!r}"
            for lang in required:
                if c.get(lang, e["key"])[0] is None:
                    errors.append(f"{where}: missing {lang}")
            for lang in c.langs(e["key"]):
                if lang == "en" or lang in languages.get("debug", []):
                    continue
                value = c.get(lang, e["key"])[0]
                if value is None:
                    continue
                problems = check_plural_shape(e, value, lang)
                for cat, v in ([] if problems else forms(value)):
                    problems += check_placeholders(source_form(e, cat), v, subset=cat == "one")
                errors += [f"{where} [{lang}]: {p}" for p in problems]
    if jsx_dir is not None:
        errors += [f"bare JSX text outside t(): {h}" for h in jsx_violations(jsx_dir)]
    return errors


# Not after `=` or `-`: `() => Promise<T>` and `a -> b<` are code, not a tag closing before text.
JSX_TEXT = re.compile(r"(?<![=-])>\s*[A-Za-z][^<{]*<")


def jsx_pending(src_dir):
    """The ui/src paths not extracted yet (l10n/pending-extraction.txt), which the JSX rule skips.
    Windows extraction lands area by area; each PR removes its lines, and the last deletes the file."""
    f = Path(src_dir) / "l10n" / "pending-extraction.txt"
    if not f.exists():
        return []
    lines = (l.strip() for l in f.read_text(encoding="utf-8").splitlines())
    return [Path(l) for l in lines if l and not l.startswith("#")]


def jsx_violations(src_dir):
    hits = []
    pending = jsx_pending(src_dir)
    for path in sorted(Path(src_dir).rglob("*.tsx")):
        rel = path.relative_to(src_dir)
        if any(rel == p or p in rel.parents for p in pending):
            continue
        prev = ""
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if JSX_TEXT.search(line) and "l10n-ignore" not in line and "l10n-ignore" not in prev:
                hits.append(f"{path}:{n}: {line.strip()}")
            prev = line
    return hits


# ---------------------------------------------------------------- extract

XLIFF_NS = {"x": "urn:oasis:names:tc:xliff:document:1.2"}


def xliff_units(xcloc, catalog_name):
    xliff = Path(xcloc)
    if xliff.is_dir():
        xliff = xliff / "Localized Contents" / "en.xliff"
    units = []
    for f in ET.parse(xliff).getroot().findall("x:file", XLIFF_NS):
        if Path(f.get("original", "")).name != catalog_name:
            continue
        for u in f.iter(f"{{{XLIFF_NS['x']}}}trans-unit"):
            src = u.find("x:source", XLIFF_NS)
            note = u.find("x:note", XLIFF_NS)
            units.append((u.get("id"), src.text or "" if src is not None else "", (note.text or "").strip() if note is not None else ""))
    return units


def merge_keys(data, units, glossary=None):
    """Keys and comments from the source code; prunes keys the code no longer has. Keeps translations."""
    dnt = set((glossary or load_glossary()).get("do_not_translate", []))
    old, new = data.get("strings", {}), {}
    for key, source, note in units:
        if "|==|" in key:  # a plural/device variation unit: the base key carries it
            key, source = key.split("|==|")[0], key.split("|==|")[0]
        e = json.loads(json.dumps(old.get(key, new.get(key, {}))))
        if note:
            e["comment"] = note
        if source != key and "variations" not in e.get("localizations", {}).get("en", {}):
            e.setdefault("localizations", {})["en"] = {"stringUnit": {"state": "new", "value": source}}
        if "shouldTranslate" not in e and (not letters(source) or source in dnt):
            e["shouldTranslate"] = False
        new[key] = e
    return {**data, "strings": new}


def extract_apple(xcloc, catalog):
    return merge_keys(read_json(catalog), xliff_units(xcloc, Path(catalog).name))


def check_apple(xcloc, catalog):
    have = read_json(catalog)
    want = extract_apple(xcloc, catalog)
    if have == want:
        return []
    a, b = have.get("strings", {}), want["strings"]
    errs = [f"{Path(catalog).name}: new source string not in the catalog: {k!r}" for k in sorted(set(b) - set(a))]
    errs += [f"{Path(catalog).name}: catalog key no longer in the source: {k!r}" for k in sorted(set(a) - set(b))]
    errs += [f"{Path(catalog).name}: {k!r} differs from the source (comment or English value)" for k in sorted(set(a) & set(b)) if a[k] != b[k]]
    errs.append(f"run: scripts/l10n/export-apple.sh <macos|ios> /tmp/loc && python3 -I scripts/l10n/translate.py extract --apple /tmp/loc/en.xcloc {catalog}")
    return errs


STRING_LITERAL = r'"((?:[^"\\]|\\.)*)"'


def windows_units(root=ROOT):
    units = []
    for d, fn, exts in WINDOWS_SOURCES:
        for path in sorted((root / d).rglob("*")):
            if path.suffix in exts:
                for m in re.finditer(rf"\b{fn}\(\s*{STRING_LITERAL}", path.read_text(encoding="utf-8")):
                    key = json.loads(f'"{m[1]}"')
                    units.append((key, key, ""))
    return units


# ---------------------------------------------------------------- translate

def anthropic_client(model, system, prompt):
    key = os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        sys.exit("ANTHROPIC_API_KEY is not set")
    body = {"model": model, "max_tokens": 16000, "system": system,
            "messages": [{"role": "user", "content": prompt}]}
    req = urllib.request.Request(
        "https://api.anthropic.com/v1/messages", data=json.dumps(body).encode(),
        headers={"x-api-key": key, "anthropic-version": "2023-06-01", "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        msg = json.load(r)
    if msg.get("stop_reason") in ("refusal", "max_tokens"):
        raise RuntimeError(f"model stopped: {msg.get('stop_reason')}")
    return "".join(b.get("text", "") for b in msg.get("content", []) if b.get("type") == "text")


def system_prompt(lang, glossary, variant):
    style = ""
    for name in (lang, base_language(lang)):
        p = L10N / "style" / f"{name}.md"
        if p.exists():
            style = p.read_text(encoding="utf-8")
            break
    rules = [
        f"You translate the user interface of Whimbrel, a meeting-notes app, from US English into {lang}.",
        "Keep every placeholder (%@, %lld, %1$@, {name} ...) exactly; positional forms may be reordered.",
        "Never translate these terms; keep them verbatim: " + ", ".join(glossary.get("do_not_translate", [])) + ".",
        "\"Ask\" is a feature name only as a noun (\"Ask is available\", \"Ask in Library\"); translate the English verb \"ask\" normally (\"Ask for a change\", \"Ask about this meeting\").",
        "Do not add markdown or HTML. Stay within max_length when one is given; otherwise stay close to the English length.",
        "For an item with \"plural\", return an object with exactly those plural categories.",
        "Reply with one JSON object mapping each item id to its translation, and nothing else.",
    ]
    if glossary.get("fixed"):
        rules.append("Fixed translations: " + json.dumps(glossary["fixed"], ensure_ascii=False))
    if variant:
        rules.append("This is a deltas-only variant: return only items whose spelling or wording differs; omit the rest.")
    return "\n".join(rules) + ("\n\n" + style if style else "")


def parse_reply(text):
    text = text[text.find("{"): text.rfind("}") + 1]
    data = json.loads(text)
    if not isinstance(data, dict):
        raise ValueError("reply is not a JSON object")
    return data


def translate(lang, catalogs, client, model, glossary, languages, memory=None):
    """Fills `lang` in `catalogs`. Translation memory first, then the model in batches of BATCH."""
    variant = lang in languages.get("variants", [])
    memory = catalogs if memory is None else memory
    tm = {}
    for c in memory:
        for e in c.entries():
            v = c.get(lang, e["key"])[0]
            if v is not None:
                tm.setdefault(json.dumps(e["source"], sort_keys=True), v)
    report = {"memory": 0, "translated": 0, "failed": []}
    todo = []
    for c in catalogs:
        for e in c.entries():
            if c.get(lang, e["key"])[0] is not None:
                continue
            hit = tm.get(json.dumps(e["source"], sort_keys=True))
            if hit is not None and not validate(e, hit, lang, glossary):
                c.set(lang, e["key"], hit, "needs_review")
                report["memory"] += 1
            else:
                todo.append(e)
    system = system_prompt(lang, glossary, variant)
    for start in range(0, len(todo), BATCH):
        batch = {str(i): e for i, e in enumerate(todo[start:start + BATCH], 1)}
        errors = {}
        for attempt in range(2):
            ids = list(batch) if attempt == 0 else list(errors)
            if not ids:
                break
            items = []
            for i in ids:
                e = batch[i]
                item = {"id": i, "key": e["key"], "english": e["source"], "comment": e["comment"]}
                if e["plural"]:
                    item["plural"] = sorted(required_plural_categories(lang))
                if e.get("limit"):
                    item["max_length"] = f"{e['limit']} characters, counting {{name}} as {MEASURED_NAME_LENGTH}"
                if attempt:
                    item["previous_error"] = "; ".join(errors[i])
                items.append(item)
            prompt = ("Your previous answer for these items was rejected; fix the errors.\n" if attempt else "") + \
                "Translate these items:\n" + json.dumps(items, ensure_ascii=False, indent=1)
            try:
                reply = parse_reply(client(model, system, prompt))
            except (ValueError, RuntimeError) as err:
                errors = {i: [f"unusable reply: {err}"] for i in ids}
                continue
            errors = {}
            for i in ids:
                e = batch[i]
                if i not in reply:
                    if not variant:
                        errors[i] = ["no translation returned"]
                    continue
                value = reply[i]
                if variant and value == e["source"]:
                    continue
                problems = validate(e, value, lang, glossary)
                if problems:
                    errors[i] = problems
                else:
                    e["catalog"].set(lang, e["key"], value, "needs_review")
                    report["translated"] += 1
        for i, problems in errors.items():
            report["failed"].append(batch[i]["key"])
            print(f"{batch[i]['catalog'].path.name}: {batch[i]['key']!r} left untranslated: {'; '.join(problems)}", file=sys.stderr)
    for c in catalogs:
        c.save()
    return report


# ---------------------------------------------------------------- review / apply

def cell(v):
    return json.dumps(v, ensure_ascii=False) if isinstance(v, dict) else v


def review(lang, n, catalogs, out):
    rows = []
    for c in catalogs:
        for e in c.entries():
            value, state = c.get(lang, e["key"])
            # Skins store no state, so every skin translation counts as unreviewed.
            if value is not None and (state == "needs_review" or c.skin):
                rows.append([e["key"], cell(e["source"]), cell(value), e["comment"]])
    rows = random.sample(rows, min(n, len(rows)))
    with open(out, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["key", "english", "translation", "comment"])
        w.writerows(rows)
    return len(rows)


def apply_csv(path, lang, catalogs, glossary):
    """Writes the reviewed rows back as "translated". Rows match on key and English, so a skin's voice
    key ("tagline") lands only in the skin it came from."""
    errors = []
    with open(path, newline="", encoding="utf-8") as f:
        for row in csv.DictReader(f):
            text = row.get("translation", "")
            if not text.strip():
                continue
            value = json.loads(text) if text.startswith('{"') else text
            hits = [e for c in catalogs for e in c.entries()
                    if e["key"] == row["key"] and cell(e["source"]) == row["english"]]
            if not hits:
                errors.append(f"{row['key']!r}: no catalog has this key with this English")
            for e in hits:
                problems = validate(e, value, lang, glossary, budget=bool(e.get("limit")))
                if problems:
                    errors.append(f"{e['catalog'].path.name}: {row['key']!r}: {'; '.join(problems)}")
                else:
                    e["catalog"].set(lang, e["key"], value, "translated")
    for c in catalogs:
        c.save()
    return errors


# ---------------------------------------------------------------- CLI

def load_glossary():
    return read_json(L10N / "glossary.json")


def load_languages():
    return read_json(L10N / "languages.json")


# ---------------------------------------------------------------- pseudo-locale

PSEUDO_LANG = "en-XA"
ACCENTS = str.maketrans("aceinousyACEINOUSY", "åçéîñöûšýÅÇÉÎÑÖÛŠÝ")


def pseudo(s):
    """Accented, 40% longer and bracketed; placeholders and {tokens} unchanged (a missed string shows unbracketed)."""
    out, last = [], 0
    for m in PLACEHOLDER.finditer(s):
        out += [s[last:m.start()].translate(ACCENTS), m.group(0)]
        last = m.end()
    out.append(s[last:].translate(ACCENTS))
    return "[" + "".join(out) + "~" * -(-len(s) * 2 // 5) + "]"


def pseudo_catalogs(catalogs, clear=False):
    """Writes (or with clear, removes) en-XA in each UI catalog. Never commit it: Release would ship en-XA.lproj."""
    for c in catalogs:
        if c.skin:
            continue
        for e in c.entries():
            if clear:
                entry = c.data["strings"].get(e["key"], {})
                if entry.get("localizations", {}).pop(PSEUDO_LANG, None) is not None:
                    c.dirty = True
                    if not entry["localizations"]:
                        del entry["localizations"]
                continue
            src = e["source"]
            c.set(PSEUDO_LANG, e["key"], {k: pseudo(v) for k, v in src.items()} if isinstance(src, dict) else pseudo(src), "translated")
        c.save()


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("extract", "check"):
        s = sub.add_parser(name)
        s.add_argument("--apple", nargs=2, metavar=("XCLOC", "CATALOG"))
    s = sub.add_parser("translate")
    s.add_argument("lang")
    s.add_argument("--model", required=True)
    s.add_argument("--catalog")
    s = sub.add_parser("review")
    s.add_argument("lang")
    s.add_argument("--sample", type=int, required=True)
    s.add_argument("--out")
    s = sub.add_parser("apply")
    s.add_argument("csv")
    s.add_argument("--lang", help="default: from the file name, review-<lang>.csv")
    s = sub.add_parser("pseudo", help="write en-XA into the UI catalogs for a local visual check; --clear removes it")
    s.add_argument("--clear", action="store_true")
    a = p.parse_args(argv)

    windows = ROOT / WINDOWS_CATALOG
    if a.cmd == "extract":
        if a.apple:
            write_json(a.apple[1], extract_apple(*a.apple))
        elif windows.exists():
            write_json(windows, merge_keys(read_json(windows), windows_units()))
        else:
            print(f"{WINDOWS_CATALOG} does not exist; nothing to extract")
        return 0
    if a.cmd == "check":
        errors = check(load_all(), load_languages(), ROOT / "apps/windows/ui/src" if windows.exists() else None)
        if a.apple:
            errors += check_apple(*a.apple)
        for e in errors:
            print(e, file=sys.stderr)
        print(f"check: {len(errors)} problem(s)")
        return 1 if errors else 0
    if a.cmd == "translate":
        all_cats = load_all()
        cats = [c for c in all_cats if a.catalog is None or c.path.resolve() == Path(a.catalog).resolve()]
        if not cats:
            sys.exit(f"not a known catalog: {a.catalog}")
        r = translate(a.lang, cats, anthropic_client, a.model, load_glossary(), load_languages(), memory=all_cats)
        print(f"{a.lang}: {r['memory']} from translation memory, {r['translated']} translated, {len(r['failed'])} failed")
        return 1 if r["failed"] else 0
    if a.cmd == "review":
        out = a.out or f"review-{a.lang}.csv"
        print(f"{review(a.lang, a.sample, load_all(), out)} rows written to {out}")
        return 0
    if a.cmd == "apply":
        lang = a.lang or re.sub(r"^review-", "", Path(a.csv).stem)
        errors = apply_csv(a.csv, lang, load_all(), load_glossary())
        for e in errors:
            print(e, file=sys.stderr)
        return 1 if errors else 0
    if a.cmd == "pseudo":
        pseudo_catalogs([c for c in load_all() if "core/" not in c.path.as_posix()], clear=a.clear)
        print(f"pseudo: {'removed' if a.clear else 'wrote'} {PSEUDO_LANG}" + ("" if a.clear else "; run `translate.py pseudo --clear` before committing"))
        return 0


if __name__ == "__main__":
    sys.exit(main())
