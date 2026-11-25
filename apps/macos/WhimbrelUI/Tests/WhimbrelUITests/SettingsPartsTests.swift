import AppKit
import Foundation
import Testing
@testable import WhimbrelUI

private let enUS = Locale(identifier: "en_US")

@Suite struct SettingsPartsTests {
    /// "Copied" shows only when the pasteboard write worked, and the default write reports whether it did.
    @Test func copiedShowsOnlyAfterASuccessfulWrite() {
        #expect(WCopyConfirmation.confirmed(id: "a", wrote: true) == "a")
        #expect(WCopyConfirmation.confirmed(id: "a", wrote: false) == nil)
        let board = NSPasteboard.withUniqueName()
        #expect(WCopyConfirmation.write("value", to: board))
        #expect(board.string(forType: .string) == "value")
        board.releaseGlobally()
    }

    // MARK: Navigation keys

    @Test func downAndUpMoveOneItemAndStopAtTheEnds() {
        #expect(WListNavigation.index(after: 0, count: 3, key: .next) == 1)
        #expect(WListNavigation.index(after: 2, count: 3, key: .next) == 2)
        #expect(WListNavigation.index(after: 1, count: 3, key: .previous) == 0)
        #expect(WListNavigation.index(after: 0, count: 3, key: .previous) == 0)
    }

    @Test func homeAndEndJump() {
        #expect(WListNavigation.index(after: 2, count: 5, key: .first) == 0)
        #expect(WListNavigation.index(after: 1, count: 5, key: .last) == 4)
    }

    @Test func withNothingSelectedDownPicksFirstAndUpPicksLast() {
        #expect(WListNavigation.index(after: nil, count: 4, key: .next) == 0)
        #expect(WListNavigation.index(after: nil, count: 4, key: .previous) == 3)
    }

    @Test func anEmptyListHasNoIndex() {
        for key in [WListKey.next, .previous, .first, .last] {
            #expect(WListNavigation.index(after: nil, count: 0, key: key) == nil)
        }
    }

    @Test func aStaleIndexIsPulledBackInRange() {
        #expect(WListNavigation.index(after: 9, count: 3, key: .previous) == 1)
        #expect(WListNavigation.index(after: -4, count: 3, key: .next) == 1)
    }

    @Test func typeAheadFindsTheNextTitleAndCycles() {
        let titles = ["General", "Profile", "Plan & billing", "Capture & Privacy"]
        #expect(WListNavigation.typeAhead("p", titles: titles, from: 0) == 1)
        #expect(WListNavigation.typeAhead("p", titles: titles, from: 1) == 2)
        #expect(WListNavigation.typeAhead("p", titles: titles, from: 2) == 1)
        #expect(WListNavigation.typeAhead("pl", titles: titles, from: 0) == 2)
        #expect(WListNavigation.typeAhead("pp", titles: titles, from: 0) == 1)
        #expect(WListNavigation.typeAhead("z", titles: titles, from: 0) == nil)
        #expect(WListNavigation.typeAhead("", titles: titles, from: 0) == nil)
    }

    @Test func segmentedHoldsTwoToFourChoicesAndFallsBackToAMenu() {
        #expect(WPickerStyle.effective(.segmented, optionCount: 3) == .segmented)
        #expect(WPickerStyle.effective(.segmented, optionCount: 4) == .segmented)
        #expect(WPickerStyle.effective(.segmented, optionCount: 5) == .menu)
        #expect(WPickerStyle.effective(.segmented, optionCount: 1) == .menu)
        #expect(WPickerStyle.effective(.menu, optionCount: 3) == .menu)
    }

    // MARK: Numbers

    private let days = WNumberRule.whole(1...3650)

    @Test func aWholeNumberInRangeIsAccepted() {
        #expect(WNumberInput.commit("30", rule: days, locale: enUS) == .set(30))
        #expect(WNumberInput.commit(" 365 ", rule: days, locale: enUS) == .set(365))
    }

    @Test func zeroAndBlankAreNotWrittenWhenTheRuleForbidsThem() {
        #expect(WNumberInput.commit("0", rule: days, locale: enUS) == nil)
        #expect(WNumberInput.parse("0", rule: days, locale: enUS) == .invalid(.belowMinimum(1)))
        #expect(WNumberInput.commit("", rule: days, locale: enUS) == nil)
        #expect(WNumberInput.parse("  ", rule: days, locale: enUS) == .invalid(.emptyNotAllowed))
    }

    @Test func textThatIsNotANumberWritesNothing() {
        #expect(WNumberInput.commit("abc", rule: days, locale: enUS) == nil)
        #expect(WNumberInput.commit("12 days", rule: days, locale: enUS) == nil)
        #expect(WNumberInput.parse("nan", rule: days, locale: enUS) == .invalid(.notANumber))
        #expect(WNumberInput.parse("inf", rule: days, locale: enUS) == .invalid(.notANumber))
    }

    @Test func outOfRangeAndFractionsAreRefused() {
        #expect(WNumberInput.parse("4000", rule: days, locale: enUS) == .invalid(.aboveMaximum(3650)))
        #expect(WNumberInput.parse("2.5", rule: days, locale: enUS) == .invalid(.tooManyDecimals(0)))
    }

    @Test func aDailyCapAllowsBlankForDefaultAndZeroToDisable() {
        let cap = WNumberRule(minimum: 0, maximum: 1_000_000, allowsEmpty: true)
        #expect(WNumberInput.commit("", rule: cap, locale: enUS) == .clear)
        #expect(WNumberInput.commit("0", rule: cap, locale: enUS) == .set(0))
        #expect(WNumberInput.commit("-5", rule: cap, locale: enUS) == nil)
    }

    // Decimal-comma locales: "12,50" is twelve and a half, never twelve hundred and fifty. Each row is
    // (text, en_US, de_DE, fr_FR, pt_BR) for a dollar field and a whole-number field. "sep" is the grouping message.
    private enum Expect: Equatable { case v(Double), empty, sep, decimals, below, notNumber }

    private func expect(_ text: String, rule: WNumberRule, locale: String) -> Expect {
        switch WNumberInput.parse(text, rule: rule, locale: Locale(identifier: locale)) {
        case .value(let v): return .v(v)
        case .empty: return .empty
        case .invalid(.separator): return .sep
        case .invalid(.tooManyDecimals): return .decimals
        case .invalid(.belowMinimum): return .below
        case .invalid: return .notNumber
        }
    }

    @Test func moneyIsReadInTheLocaleAndNeverGuessed() {
        let money = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 2)
        let rows: [(String, [String: Expect])] = [
            ("5", ["en_US": .v(5), "de_DE": .v(5), "fr_FR": .v(5), "pt_BR": .v(5)]),
            ("5.5", ["en_US": .v(5.5), "de_DE": .v(5.5), "fr_FR": .v(5.5), "pt_BR": .v(5.5)]),
            ("5,5", ["en_US": .sep, "de_DE": .v(5.5), "fr_FR": .v(5.5), "pt_BR": .v(5.5)]),
            ("12,50", ["en_US": .sep, "de_DE": .v(12.5), "fr_FR": .v(12.5), "pt_BR": .v(12.5)]),
            ("12.50", ["en_US": .v(12.5), "de_DE": .v(12.5), "fr_FR": .v(12.5), "pt_BR": .v(12.5)]),
            ("1,000", ["en_US": .sep, "de_DE": .decimals, "fr_FR": .decimals, "pt_BR": .decimals]),
            ("1.000", ["en_US": .decimals, "de_DE": .sep, "fr_FR": .decimals, "pt_BR": .sep]),
            ("1,000.50", ["en_US": .sep, "de_DE": .sep, "fr_FR": .sep, "pt_BR": .sep]),
            ("1.000,50", ["en_US": .sep, "de_DE": .sep, "fr_FR": .sep, "pt_BR": .sep]),
            ("$5", ["en_US": .v(5), "de_DE": .v(5), "fr_FR": .v(5), "pt_BR": .v(5)]),
            ("", ["en_US": .empty, "de_DE": .empty, "fr_FR": .empty, "pt_BR": .empty]),
            ("0", ["en_US": .v(0), "de_DE": .v(0), "fr_FR": .v(0), "pt_BR": .v(0)]),
            ("-1", ["en_US": .below, "de_DE": .below, "fr_FR": .below, "pt_BR": .below]),
            ("abc", ["en_US": .notNumber, "de_DE": .notNumber, "fr_FR": .notNumber, "pt_BR": .notNumber]),
        ]
        for (text, expected) in rows {
            for (locale, want) in expected {
                #expect(expect(text, rule: money, locale: locale) == want, "\(text) in \(locale)")
            }
        }
    }

    @Test func wholeNumberRowsRefuseAnySeparator() {
        let whole = WNumberRule(minimum: 0, maximum: 1_000_000, allowsEmpty: true)
        let rows: [(String, [String: Expect])] = [
            ("5", ["en_US": .v(5), "de_DE": .v(5), "fr_FR": .v(5), "pt_BR": .v(5)]),
            ("5.5", ["en_US": .decimals, "de_DE": .decimals, "fr_FR": .decimals, "pt_BR": .decimals]),
            ("5,5", ["en_US": .sep, "de_DE": .decimals, "fr_FR": .decimals, "pt_BR": .decimals]),
            ("12,50", ["en_US": .sep, "de_DE": .decimals, "fr_FR": .decimals, "pt_BR": .decimals]),
            ("1,000", ["en_US": .sep, "de_DE": .decimals, "fr_FR": .decimals, "pt_BR": .decimals]),
            ("1.000", ["en_US": .decimals, "de_DE": .sep, "fr_FR": .decimals, "pt_BR": .sep]),
            ("1,000.50", ["en_US": .sep, "de_DE": .sep, "fr_FR": .sep, "pt_BR": .sep]),
            ("1.000,50", ["en_US": .sep, "de_DE": .sep, "fr_FR": .sep, "pt_BR": .sep]),
            ("$5", ["en_US": .v(5), "de_DE": .v(5), "fr_FR": .v(5), "pt_BR": .v(5)]),
            ("0", ["en_US": .v(0), "de_DE": .v(0), "fr_FR": .v(0), "pt_BR": .v(0)]),
            ("-1", ["en_US": .below, "de_DE": .below, "fr_FR": .below, "pt_BR": .below]),
            ("abc", ["en_US": .notNumber, "de_DE": .notNumber, "fr_FR": .notNumber, "pt_BR": .notNumber]),
        ]
        for (text, expected) in rows {
            for (locale, want) in expected {
                #expect(expect(text, rule: whole, locale: locale) == want, "\(text) in \(locale)")
            }
        }
        // A separator never becomes a bigger or smaller number: the only values that come out are the typed digits.
        #expect(WNumberInput.commit("1,000", rule: whole, locale: Locale(identifier: "en_US")) == nil)
        #expect(WNumberInput.commit("1.000", rule: whole, locale: Locale(identifier: "de_DE")) == nil)
    }

    @Test func aSpaceOrTheLocalesGroupingMarkInsideANumberIsRefused() {
        let money = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 2)
        for locale in ["en_US", "de_DE", "fr_FR", "pt_BR"] {
            #expect(expect("1 000", rule: money, locale: locale) == .sep)
            #expect(expect("1\u{202F}000", rule: money, locale: locale) == .sep)
        }
        #expect(expect("1'000", rule: money, locale: "de_CH") == .sep)
    }

    @Test func aFieldShowsAStoredFigureInTheLocaleAndReadsItBack() {
        let money = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 2)
        for locale in ["en_US", "de_DE", "fr_FR", "pt_BR"] {
            let l = Locale(identifier: locale)
            let shown = WNumberInput.text(for: 12.5, rule: money, locale: l)
            #expect(WNumberInput.parse(shown, rule: money, locale: l) == .value(12.5), "\(locale) \(shown)")
            let big = WNumberInput.text(for: 1250.5, rule: money, locale: l)
            #expect(WNumberInput.parse(big, rule: money, locale: l) == .value(1250.5), "\(locale) \(big)")
        }
        #expect(WNumberInput.text(for: 12.5, rule: money, locale: Locale(identifier: "de_DE")) == "12,50")
    }

    /// Digits are Latin in every language, so the parser reads what the formatter wrote (it refused "\u{0661}\u{0662}\u{066B}\u{0665}\u{0660}").
    @Test func aFigureIsShownWithLatinDigitsInEveryLocaleAndReadsBack() {
        let money = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 2)
        let whole = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 0)
        for id in ["ar_EG", "ar_SA", "fa_IR", "ur_PK", "bn_BD", "mr_IN", "my_MM", "en_US", "de_DE", "hi_IN"] {
            let l = Locale(identifier: id)
            for (value, rule) in [(12.5, money), (1250.5, money), (7, whole), (0, money)] {
                let shown = WNumberInput.text(for: value, rule: rule, locale: l)
                #expect(shown.unicodeScalars.allSatisfy { $0.isASCII }, "\(id) \(shown)")
                #expect(WNumberInput.parse(shown, rule: rule, locale: l) == .value(value), "\(id) \(shown)")
            }
        }
        // A person typing Arabic-Indic digits is read too.
        #expect(WNumberInput.parse("\u{0661}\u{0662}", rule: whole, locale: Locale(identifier: "ar_EG")) == .value(12))
    }

    @Test func currencyTakesADollarSignAndTwoDecimalsButNoGrouping() {
        let budget = WNumberRule(minimum: 0, maximum: 100_000, allowsEmpty: true, fractionDigits: 2)
        #expect(WNumberInput.commit("$12.50", rule: budget, locale: enUS) == .set(12.5))
        #expect(WNumberInput.commit("1,200", rule: budget, locale: enUS) == nil)
        #expect(WNumberInput.commit("", rule: budget, locale: enUS) == .clear)
        #expect(WNumberInput.parse("3.141", rule: budget, locale: enUS) == .invalid(.tooManyDecimals(2)))
        #expect(WNumberInput.text(for: 12.5, rule: budget, locale: enUS) == "12.50")
        #expect(WNumberInput.text(for: nil, rule: budget, locale: enUS) == "")
        #expect(WNumberInput.text(for: 30, rule: days, locale: enUS) == "30")
    }

    @Test func theCompanionCheckboxKeepsTheStoredNumberOnScreen() {
        var stepper = WNumberStepper(value: 30, range: 1...3650, companionOn: false)
        #expect(stepper.displayedValue == 30 && stepper.isStepperEnabled)
        stepper.companionOn = true
        // "Keep forever" is on: the stepper is off, the 30 stays, and no step writes anything.
        #expect(stepper.displayedValue == 30)
        #expect(!stepper.isStepperEnabled && !stepper.canIncrease && !stepper.canDecrease)
        #expect(stepper.stepped(1) == nil && stepper.stepped(-1) == nil)
        stepper.companionOn = false
        #expect(stepper.stepped(1) == 31)
    }

    @Test func aStepperClampsAtItsEndsAndUsesItsStep() {
        #expect(WNumberStepper(value: 1, range: 1...10).stepped(-1) == nil)
        #expect(WNumberStepper(value: 10, range: 1...10).stepped(1) == nil)
        #expect(WNumberStepper(value: 5, range: 5...60, step: 5).stepped(1) == 10)
        #expect(WNumberStepper(value: 58, range: 5...60, step: 5).stepped(1) == 60)
        // An old stored value outside the range shows clamped and writes nothing by itself.
        #expect(WNumberStepper(value: 9999, range: 1...3650).displayedValue == 3650)
        #expect(WNumberStepper(value: 5, range: 1...10).stepped(0) == nil)
    }

    // MARK: Secrets

    @Test func aSecretRowMovesFromEmptyToSavedToReplacing() {
        #expect(WSecretMachine.phase(isSaved: false, replacing: false) == .empty)
        #expect(WSecretMachine.phase(isSaved: false, replacing: false, draft: "sk-example") == .editing(replacing: false))
        #expect(WSecretMachine.phase(isSaved: true, replacing: false) == .saved)
        #expect(WSecretMachine.phase(isSaved: true, replacing: true) == .editing(replacing: true))
    }

    @Test func aSavedSecretIsNeverShownOnlyMasked() {
        // The row takes no secret, so there is nothing to echo. The mask is one fixed length for every key.
        #expect(WSecretMachine.mask.allSatisfy { $0 == "\u{2022}" })
        #expect(WSecretMachine.mask.count == 12)
        #expect(WSecretMachine.draftAfterSave().isEmpty)
    }

    @Test func saveNeedsRealContentAndTrimsAPastedKey() {
        #expect(!WSecretMachine.canSave(draft: ""))
        #expect(!WSecretMachine.canSave(draft: "  \n "))
        #expect(WSecretMachine.valueToSave(draft: "  sk-example-not-a-key\n") == "sk-example-not-a-key")
        #expect(WSecretMachine.valueToSave(draft: "   ") == nil)
        #expect(WSecretMachine.canRemove(isSaved: true) && !WSecretMachine.canRemove(isSaved: false))
    }

    @Test func aFailedSaveKeepsTheTypedKeyAndASavedOneClearsIt() {
        var draft = WSecretDraft(text: "  sk-example-not-a-key\n", replacing: true)
        #expect(draft.valueToSave == "sk-example-not-a-key")
        draft.finishSave(succeeded: false)
        #expect(draft.text == "  sk-example-not-a-key\n")
        #expect(draft.replacing)
        draft.finishSave(succeeded: true)
        #expect(draft.text.isEmpty)
        #expect(!draft.replacing)
    }

    @Test func cancelRemoveAndLeavingTheScreenAllDropTheTypedKey() {
        for action in [{ (d: inout WSecretDraft) in d.cancel() }, { (d: inout WSecretDraft) in d.remove() }, { (d: inout WSecretDraft) in d.disappear() }] {
            var draft = WSecretDraft(text: "typed", replacing: true)
            action(&draft)
            #expect(draft == WSecretDraft())
        }
        // Nothing to save while only spaces are typed.
        #expect(WSecretDraft(text: "   ").valueToSave == nil)
    }

    @Test func aFactEditLeftOpenIsSavedWhenChangedAndNotBlank() {
        #expect(WFactEdit.textToSaveOnLeaving(original: "Works late", draft: "Works late on Thursdays") == "Works late on Thursdays")
        #expect(WFactEdit.textToSaveOnLeaving(original: "Works late", draft: "Works late") == nil)
        #expect(WFactEdit.textToSaveOnLeaving(original: "Works late", draft: "   ") == nil)
        #expect(WFactEdit.textToSaveOnLeaving(original: nil, draft: "anything") == nil)
    }

    // MARK: Typed confirmation

    @Test func theConfirmButtonNeedsTheExactWord() {
        #expect(WTypedConfirmation.matches(typed: "DELETE", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: " DELETE ", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: "DELETE\n", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: "delete", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: "DELET", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: "", word: "DELETE"))
        #expect(!WTypedConfirmation.matches(typed: "", word: ""))
    }

    // MARK: List add

    @Test func addingTrimsAndRefusesBlank() {
        let rule = WListAddRule()
        #expect(rule.validate("  example.com  ", existing: []) == .accepted("example.com"))
        #expect(rule.validate("   ", existing: []) == .rejected(.blank))
    }

    @Test func duplicatesIgnoreCaseWhenAsked() {
        #expect(WListAddRule().validate("Example.com", existing: ["example.com"]) == .rejected(.duplicate))
        #expect(WListAddRule(rejectsDuplicates: false).validate("Example.com", existing: ["example.com"]) == .accepted("Example.com"))
    }

    @Test func lengthAndCountLimits() {
        let rule = WListAddRule(maxLength: 5, maxCount: 2)
        #expect(rule.validate("toolong", existing: []) == .rejected(.tooLong(5)))
        #expect(rule.validate("ok", existing: ["a", "b"]) == .rejected(.full(2)))
        #expect(rule.validate("ok", existing: ["a"]) == .accepted("ok"))
    }

    // MARK: Middle truncation

    @Test func aLongPathLosesItsMiddleAndKeepsBothEnds() {
        let path = "/Users/alex/Library/Application Support/Whimbrel/Recordings/Audio"
        let short = WMiddleTruncation.truncate(path, limit: 40)
        #expect(short.count == 40)
        #expect(short.hasPrefix("/Users/alex/"))
        #expect(short.hasSuffix("Recordings/Audio"))
        #expect(short.contains("\u{2026}"))
    }

    @Test func aShortValueIsLeftAlone() {
        #expect(WMiddleTruncation.truncate("v1.2.3", limit: 30) == "v1.2.3")
        #expect(WMiddleTruncation.truncate("abcdef", limit: 6) == "abcdef")
        #expect(WMiddleTruncation.truncate("abcdefg", limit: 2) == "abcdefg")
    }

    // MARK: Shortcut

    @Test func aShortcutSplitsIntoKeycaps() {
        #expect(WShortcutDisplay.keys(from: "\u{2303} \u{2325} Space") == ["\u{2303}", "\u{2325}", "Space"])
        #expect(WShortcutDisplay.keys(from: nil).isEmpty)
        #expect(WShortcutDisplay.keys(from: "  ").isEmpty)
    }
}
