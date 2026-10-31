"""Tests for translate.py. Run: python3 -I -m unittest discover -s scripts/l10n -v"""
import json
import os
import re
import shutil
import sys
import tempfile
import unittest
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import translate  # noqa: E402

FIXTURES = HERE / "tests" / "fixtures"
GLOSSARY = {"do_not_translate": ["Whimbrel", "Away Mode", "Today"], "fixed": {}}


def xcstrings(strings):
    return {"sourceLanguage": "en", "strings": strings, "version": "1.0"}


def unit(value, state="translated"):
    return {"stringUnit": {"state": state, "value": value}}


class Tmp(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)

    def write(self, name, data):
        p = self.dir / name
        p.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return p

    def catalog(self, strings, name="Localizable.xcstrings"):
        return translate.Catalog.load(self.write(name, xcstrings(strings)))

    def entry(self, source, key=None, **kw):
        c = self.catalog({key or source: {}})
        e = c.entries()[0]
        e.update(kw)
        return e


class CheckTests(Tmp):
    def test_check_flags_missing_key_for_shipped_lang(self):
        c = self.catalog({
            "Open": {"localizations": {"fr": unit("Ouvrir")}},
            "Close": {},
            "—": {"shouldTranslate": False},
        })
        errors = translate.check([c], {"shipped": ["fr"], "variants": [], "debug": []})
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("Close", errors[0])
        self.assertIn("fr", errors[0])

    def test_check_ignores_variant_deltas(self):
        c = self.catalog({"Color": {"localizations": {"en-GB": unit("Colour")}}, "Open": {}})
        langs = {"shipped": ["en-GB", "en-CA"], "variants": ["en-GB", "en-CA"], "debug": ["en-XA"]}
        self.assertEqual(translate.check([c], langs), [])

    def test_check_flags_placeholder_mismatch_in_catalog(self):
        c = self.catalog({"Hello, %@": {"localizations": {"fr": unit("Bonjour")}}})
        errors = translate.check([c], {"shipped": [], "variants": [], "debug": []})
        self.assertTrue(any("placeholder" in e for e in errors), errors)
        c = self.catalog({"%lld from %@": {"localizations": {"fr": unit("%@ : %lld")}}}, "B.xcstrings")
        self.assertTrue(translate.check([c], {"shipped": [], "variants": [], "debug": []}))


class ValidationTests(Tmp):
    def test_placeholder_set_mismatch_rejected(self):
        for source, good, bad in [
            ("Open %@", "Ouvrir %@", "Ouvrir"),
            ("%lld notes", "%lld remarques", "%@ remarques"),
            ("Hi {name}", "Salut {name}", "Salut {nom}"),
        ]:
            e = self.entry(source)
            self.assertEqual(translate.validate(e, good, "fr", GLOSSARY), [], source)
            errs = translate.validate(e, bad, "fr", GLOSSARY)
            self.assertTrue(any("placeholder" in x for x in errs), (source, errs))
        # Positional forms count as the same placeholder.
        e = self.entry("%@ · %@")
        self.assertEqual(translate.validate(e, "%2$@ · %1$@", "fr", GLOSSARY), [])

    def test_placeholder_types_checked_per_argument(self):
        def errs(source, value):
            return translate.validate(self.entry(source), value, "fr", GLOSSARY)
        # Reordering unnumbered specifiers would read an integer as an object at runtime.
        self.assertTrue(errs("%lld notes from %@", "%@ : %lld remarques"))
        # Positions make a reorder safe.
        self.assertEqual(errs("%1$lld notes from %2$@", "%2$@ : %1$lld remarques"), [])
        self.assertEqual(errs("%lld notes from %@", "%2$@ : %1$lld remarques"), [])
        self.assertTrue(errs("%1$lld notes from %2$@", "%1$@ : %2$lld remarques"))
        self.assertTrue(errs("%lld notes", "%ld remarques"))
        # Unrecognized sequences (%n writes memory; * reads an extra argument) are rejected.
        self.assertTrue(any("unrecognized" in x for x in errs("%lld notes", "%lld remarques%n")))
        self.assertTrue(any("unrecognized" in x for x in errs("%lld notes", "%*lld remarques")))
        self.assertEqual(errs("%lld notes", "%lld notes 100%%"), [])
        # {name} placeholders match by name.
        self.assertTrue(errs("Hi {name}, {n} new", "Salut, {n} nouveaux"))
        self.assertTrue(errs("Hi {name}", "Salut {nom}"))
        self.assertEqual(errs("Hi {name}, {n} new", "{n} nouveaux, {name}"), [])
        # Not printf (core's chrono day pattern): fields may move but not change.
        self.assertEqual(errs("%A, %B %-d", "%A %-d %B"), [])
        self.assertTrue(errs("%A, %B %-d", "%A %-d"))

    def test_plural_one_form_may_drop_only_trailing_count(self):
        c = self.catalog({"%lld meetings": {"localizations": {"en": {"variations": {"plural": {
            "one": unit("%lld meeting", "new"), "other": unit("%lld meetings", "new")}}}}}})
        e = c.entries()[0]
        self.assertEqual(translate.validate(e, {"one": "une réunion", "other": "%lld réunions"}, "fr", GLOSSARY), [])
        self.assertTrue(translate.validate(e, {"one": "%@ réunion", "other": "%lld réunions"}, "fr", GLOSSARY))

    def test_do_not_translate_terms_must_survive(self):
        e = self.entry("Turn on Away Mode")
        self.assertEqual(translate.validate(e, "Activer Away Mode", "fr", GLOSSARY), [])
        errs = translate.validate(e, "Activer le mode absence", "fr", GLOSSARY)
        self.assertTrue(any("Away Mode" in x for x in errs), errs)

    def test_ask_stays_english_only_as_the_feature_name(self):
        g = {"do_not_translate": ["Ask"], "not_when": {"Ask": r"\bAsk (a|about|for|if|the|Whimbrel|\{)"}}
        self.assertEqual(translate.validate(self.entry("Ask for a change…"), "Demander un changement…", "fr", g), [])
        self.assertEqual(translate.validate(self.entry("Ask Whimbrel"), "Interroger Whimbrel", "fr", g), [])
        errs = translate.validate(self.entry("Ask in Library"), "Demander dans la bibliothèque", "fr", g)
        self.assertTrue(any("Ask" in x for x in errs), errs)

    def test_plural_categories_required(self):
        self.assertEqual(translate.plural_category("fr", 0), "one")
        self.assertEqual(translate.plural_category("fr", 1), "one")
        self.assertEqual(translate.plural_category("fr", 2), "other")
        self.assertEqual(translate.plural_category("en", 0), "other")
        self.assertEqual(translate.plural_category("es", 0), "other")
        c = self.catalog({"%lld meetings": {"localizations": {"en": {"variations": {"plural": {
            "one": unit("%lld meeting", "new"), "other": unit("%lld meetings", "new")}}}}}})
        e = c.entries()[0]
        self.assertTrue(e["plural"])
        ok = {"one": "%lld réunion", "other": "%lld réunions"}
        self.assertEqual(translate.validate(e, ok, "fr", GLOSSARY), [])
        errs = translate.validate(e, {"other": "%lld réunions"}, "fr", GLOSSARY)
        self.assertTrue(any("one" in x for x in errs), errs)
        self.assertTrue(translate.validate(e, "%lld réunions", "fr", GLOSSARY))
        # Core's {count} keys are plural too.
        e = self.entry("{count} threads need a reply")
        self.assertTrue(e["plural"])

    def test_length_budget(self):
        e = self.entry("x" * 20)
        self.assertEqual(translate.validate(e, "y" * 30, "fr", GLOSSARY), [])
        errs = translate.validate(e, "y" * 31, "fr", GLOSSARY)
        self.assertTrue(any("long" in x for x in errs), errs)
        # A skin voice line is held to its VoiceKey limit, with {name} as 30 characters.
        skin = self.write("s.json", {"id": "s", "voice": {"greetingMorning": "Good morning, {name}"}})
        e = translate.Catalog.load(skin).entries()[0]
        limit = translate.VOICE_LIMITS["greetingMorning"]
        fits = "B" * (limit - translate.MEASURED_NAME_LENGTH - 1) + " {name}"  # exactly the limit
        self.assertEqual(translate.validate(e, fits, "fr", GLOSSARY), [])
        errs = translate.validate(e, "B" + fits, "fr", GLOSSARY)
        self.assertTrue(any("long" in x for x in errs), errs)

    def test_no_new_markdown_or_html(self):
        e = self.entry("Save the file")
        for bad in ["Enregistrer **le** fichier", "Enregistrer <b>le</b> fichier",
                    "Enregistrer `le` fichier", "[Enregistrer](x) le fichier"]:
            self.assertTrue(translate.validate(e, bad, "fr", GLOSSARY), bad)
        e = self.entry("Use **bold** here")
        self.assertEqual(translate.validate(e, "Utiliser **gras** ici", "fr", GLOSSARY), [])

    def test_voice_limits_match_rust(self):
        src = (translate.ROOT / "core/skins/src/format.rs").read_text(encoding="utf-8")
        names = dict(re.findall(r"VoiceKey::(\w+)\s*=>\s*\"(\w+)\"", src))
        body = src[src.index("pub fn limit"):]
        body = body[:body.index("pub fn", 10)]
        limits = {}
        for keys, n in re.findall(r"((?:\s*\|?\s*VoiceKey::\w+)+)\s*=>\s*\{?\s*(\d+)", body):
            for k in re.findall(r"VoiceKey::(\w+)", keys):
                limits[names[k]] = int(n)
        self.assertEqual(translate.VOICE_LIMITS, limits)
        m = re.search(r"MEASURED_NAME_LENGTH: usize = (\d+)", src)
        self.assertEqual(translate.MEASURED_NAME_LENGTH, int(m.group(1)))


class TranslateTests(Tmp):
    def no_network(self):
        def boom(*a, **k):
            raise AssertionError("network used")
        orig = urllib.request.urlopen
        urllib.request.urlopen = boom
        self.addCleanup(setattr, urllib.request, "urlopen", orig)

    def test_translation_memory_reuses_identical_source(self):
        self.no_network()
        a = self.catalog({"Cancel": {"localizations": {"fr": unit("Annuler")}}}, "A.xcstrings")
        b = self.catalog({"Cancel": {}}, "B.xcstrings")

        def client(*a, **k):
            raise AssertionError("model called for a string in translation memory")

        report = translate.translate("fr", [b], client, "fake", GLOSSARY, {"variants": []}, memory=[a, b])
        self.assertEqual(report["memory"], 1)
        data = json.loads(b.path.read_text(encoding="utf-8"))
        self.assertEqual(data["strings"]["Cancel"]["localizations"]["fr"], unit("Annuler", "needs_review"))

    def test_translate_uses_injected_client(self):
        self.no_network()
        c = self.catalog({
            "Open Today": {},
            "%lld meetings": {"localizations": {"en": {"variations": {"plural": {
                "one": unit("%lld meeting", "new"), "other": unit("%lld meetings", "new")}}}}},
            "Bad": {},
        })
        calls = []

        def client(model, system, prompt):
            calls.append((model, system, prompt))
            items = json.loads(prompt[prompt.index("["):])
            out = {}
            for it in items:
                if it["english"] == "Open Today":
                    out[it["id"]] = "Ouvrir Today"
                elif isinstance(it["english"], dict):
                    out[it["id"]] = {"one": "%lld réunion", "other": "%lld réunions"}
                else:
                    out[it["id"]] = "Mauvais **gras**"  # always invalid
            return "```json\n" + json.dumps(out) + "\n```"

        report = translate.translate("fr", [c], client, "fake-model", GLOSSARY, {"variants": []})
        self.assertEqual(calls[0][0], "fake-model")
        self.assertEqual(len(calls), 2)  # one retry, for "Bad" only
        self.assertIn("Bad", calls[1][2])
        self.assertEqual(report["failed"], ["Bad"])
        data = json.loads(c.path.read_text(encoding="utf-8"))["strings"]
        self.assertEqual(data["Open Today"]["localizations"]["fr"], unit("Ouvrir Today", "needs_review"))
        plural = data["%lld meetings"]["localizations"]["fr"]["variations"]["plural"]
        self.assertEqual(plural["one"], unit("%lld réunion", "needs_review"))
        self.assertNotIn("fr", data["Bad"].get("localizations", {}))

    def test_review_and_apply_round_trip(self):
        c = self.catalog({"Open": {"localizations": {"fr": unit("Ouvrir", "needs_review")}}})
        out = self.dir / "review-fr.csv"
        translate.review("fr", 5, [c], out)
        text = out.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("key,english,translation,comment"))
        out.write_text(text.replace("Ouvrir", "Ouvrir maintenant"), encoding="utf-8")
        errors = translate.apply_csv(out, "fr", [translate.Catalog.load(c.path)], GLOSSARY)
        self.assertEqual(errors, [])
        data = json.loads(c.path.read_text(encoding="utf-8"))
        self.assertEqual(data["strings"]["Open"]["localizations"]["fr"], unit("Ouvrir maintenant"))
        out.write_text("key,english,translation,comment\n%lld from %@,%lld from %@,%@ : %lld,\n", encoding="utf-8")
        c2 = self.catalog({"%lld from %@": {}}, "C.xcstrings")
        self.assertTrue(translate.apply_csv(out, "fr", [c2], GLOSSARY))


class SourceTests(Tmp):
    def test_jsx_text_outside_t_flagged_and_ignore_respected(self):
        hits = translate.jsx_violations(FIXTURES)
        self.assertEqual(len(hits), 1, hits)
        self.assertIn("App.tsx:9", hits[0])

    def test_jsx_arrow_type_is_not_text(self):
        (self.dir / "Card.tsx").write_text(
            "const run = (call: () => Promise<Accounts>) => {};\n<p>Bare text</p>\n", encoding="utf-8")
        hits = translate.jsx_violations(self.dir)
        self.assertEqual(len(hits), 1, hits)
        self.assertIn("Card.tsx:2", hits[0])

    def jsx_tree(self, pending=None):
        for area in ("done", "todo"):
            (self.dir / area).mkdir()
            shutil.copy(FIXTURES / "App.tsx", self.dir / area / "App.tsx")
        if pending is not None:
            (self.dir / "l10n").mkdir()
            (self.dir / "l10n" / "pending-extraction.txt").write_text(pending, encoding="utf-8")

    def test_jsx_pending_list_skips_listed_paths(self):
        self.jsx_tree("# not extracted yet\ntodo\n\n")
        hits = translate.jsx_violations(self.dir)
        self.assertEqual(len(hits), 1, hits)
        self.assertIn("done", hits[0])

    def test_jsx_without_pending_list_checks_everything(self):
        self.jsx_tree()
        self.assertEqual(len(translate.jsx_violations(self.dir)), 2)

    def test_extract_apple_adds_and_prunes(self):
        cat = self.dir / "Localizable.xcstrings"
        shutil.copy(FIXTURES / "mini.xcstrings", cat)
        xliff = FIXTURES / "en.xliff"
        self.assertTrue(translate.check_apple(xliff, cat))  # "New string" is not in the catalog yet
        merged = translate.extract_apple(xliff, cat)
        s = merged["strings"]
        self.assertEqual(set(s), {"%lld meetings", "%@ · %@", "Hello, %@", "Kept", "New string"})
        self.assertEqual(s["New string"]["comment"], "Shown on the new screen")
        self.assertEqual(s["Kept"]["localizations"]["fr"]["stringUnit"]["value"], "Gardé")
        self.assertIn("variations", s["%lld meetings"]["localizations"]["en"])
        self.assertEqual(s["%@ · %@"]["localizations"]["en"]["stringUnit"]["value"], "%1$@ · %2$@")
        self.assertIs(s["%@ · %@"]["shouldTranslate"], False)
        self.assertNotIn("NSMicrophoneUsageDescription", s)
        translate.write_json(cat, merged)
        self.assertEqual(translate.check_apple(xliff, cat), [])
        self.assertIn('"New string" : {', cat.read_text(encoding="utf-8"))



class PseudoTests(Tmp):
    def test_pseudo_preserves_placeholders_and_pads(self):
        for s in ["Hi {name}", "%lld days", "Saved. %1$@ and %2$lld"]:
            p = translate.pseudo(s)
            self.assertTrue(p.startswith("[") and p.endswith("]"), p)
            self.assertEqual(translate.placeholders(p), translate.placeholders(s), p)
            self.assertGreaterEqual(len(p), 1.4 * len(s), p)
        self.assertIn("{name}", translate.pseudo("Hi {name}"))
        self.assertNotIn("a", translate.pseudo("abc").strip("[]~"))

    def test_pseudo_writes_ui_catalogs_not_core(self):
        ui = self.catalog({"Hello": {}, "%lld days": {"localizations": {"en": {"variations": {"plural": {
            "one": {"stringUnit": {"state": "translated", "value": "%lld day"}},
            "other": {"stringUnit": {"state": "translated", "value": "%lld days"}}}}}}}})
        translate.pseudo_catalogs([ui])
        value, state = ui.get("en-XA", "Hello")
        self.assertEqual((value, state), (translate.pseudo("Hello"), "translated"))
        self.assertEqual(ui.get("en-XA", "%lld days")[0]["one"], translate.pseudo("%lld day"))
        translate.pseudo_catalogs([ui], clear=True)
        self.assertEqual(ui.get("en-XA", "Hello"), (None, None))
        self.assertNotIn("localizations", ui.data["strings"]["Hello"])

if __name__ == "__main__":
    unittest.main()
