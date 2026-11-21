import SwiftUI

/// The small numbered circle on a line Whimbrel wrote. A button: activating it selects its source.
struct WSourceMark: View {
    @Environment(\.wStrings) private var strings
    let number: Int
    var selected = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text("\(number)")
                .font(.system(size: 12, weight: .bold)).monospacedDigit()
                .foregroundStyle(selected ? Color.theme(.backgroundCard) : WFixed.aiInk.color)
                .frame(width: 22, height: 22)
                .background(selected ? WFixed.aiInk.color : Color.theme(.backgroundCard), in: Circle())
                .overlay(Circle().strokeBorder(WFixed.aiInk.color, lineWidth: 1.5))
                .wTapTarget()
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(strings.sourceNumber(number))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// One block of a note. Your words are in the text color; Whimbrel's are in AI ink with a left
/// rule and a source mark, so the two never look alike.
struct WNoteBlock: View {
    enum Kind { case heading, paragraph, bullet }
    enum Author { case you, whimbrel }
    @Environment(\.wStrings) private var strings
    let kind: Kind
    let author: Author
    let text: String
    var source: Int?
    var selected = false
    var onSource: (() -> Void)?

    var body: some View {
        let ink = author == .whimbrel ? WFixed.aiInk.color : Color.theme(.textPrimary)
        HStack(alignment: .top, spacing: WTokens.Space.sm) {
            if author == .whimbrel, kind != .heading {
                Rectangle().fill(WFixed.aiInk.color).frame(width: 2).accessibilityHidden(true)
            }
            if kind == .bullet {
                Circle().fill(ink).frame(width: 5, height: 5).padding(.top, 10).accessibilityHidden(true)
            }
            Group {
                if kind == .heading {
                    Text(text.uppercased()).wType(WTokens.TypeScale.caption).fontWeight(.bold).tracking(0.6)
                        .accessibilityAddTraits(.isHeader)
                } else {
                    Text(text).wReading(WTokens.TypeScale.reading)
                        .accessibilityLabel(author == .whimbrel ? strings.whimbrelWrote(text) : text)
                }
            }
            .foregroundStyle(kind == .heading && author == .you ? Color.theme(.textSecondary) : ink)
            .frame(maxWidth: .infinity, alignment: .leading)
            if let source, let onSource {
                WSourceMark(number: source, selected: selected, action: onSource)
            }
        }
        .padding(.vertical, WTokens.Space.xs)
        .padding(.horizontal, selected ? WTokens.Space.sm : 0)
        .background(selected ? WFixed.aiTint.color : .clear)
    }
}

/// Where a line came from: its number, kind, title, time, a short quote and Open.
struct WSourceCard: View {
    enum Kind { case transcriptMoment, seenOnScreen }
    @Environment(\.wStrings) private var strings
    let number: Int
    let kind: Kind
    let title: String
    let time: String
    var quote: String?
    var selected = false
    let onOpen: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
            HStack(spacing: WTokens.Space.sm) {
                Text("\(number)")
                    .font(.system(size: 12, weight: .bold)).monospacedDigit()
                    .foregroundStyle(Color.theme(.backgroundCard))
                    .frame(width: 22, height: 22)
                    .background(WFixed.aiInk.color, in: Circle())
                    .accessibilityLabel(strings.sourceNumber(number))
                Text("\(kind == .transcriptMoment ? strings.transcriptMoment : strings.seenOnScreen) · \(time)")
                    .wType(WTokens.TypeScale.caption).foregroundStyle(Color.theme(.textSecondary))
            }
            Text(title).wType(WTokens.TypeScale.subtitle).foregroundStyle(Color.theme(.textPrimary))
            if let quote {
                Text(quote).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textPrimary))
            }
            Button(strings.open, action: onOpen).buttonStyle(.w(.secondary))
        }
        .padding(WTokens.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(tint: selected ? WFixed.aiTint.color : nil)
        .overlay {
            if selected {
                RoundedRectangle(cornerRadius: WTokens.Radius.card, style: .continuous).strokeBorder(WFixed.aiInk.color, lineWidth: 1.5)
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// An action item: a checkbox, the text, who owns it and when it is due. The whole row toggles.
struct WActionItemRow: View {
    @Environment(\.wStrings) private var strings
    let text: String
    var owner: String?
    var due: String?
    let done: Bool
    let onToggle: () -> Void

    var body: some View {
        Button(action: onToggle) {
            HStack(alignment: .top, spacing: WTokens.Space.md) {
                Image(systemName: done ? "checkmark.square.fill" : "square")
                    .font(.title3)
                    .foregroundStyle(done ? Color.theme(.accentPrimary) : Color.theme(.textSecondary))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
                    Text(text).wType(WTokens.TypeScale.body).strikethrough(done)
                        .foregroundStyle(done ? Color.theme(.textSecondary) : Color.theme(.textPrimary))
                    if owner != nil || due != nil {
                        Text([owner, due].compactMap { $0 }.joined(separator: " · "))
                            .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(minHeight: WTokens.Size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityValue(done ? strings.done : strings.notDone)
        .accessibilityAddTraits(.isToggle)
    }
}
