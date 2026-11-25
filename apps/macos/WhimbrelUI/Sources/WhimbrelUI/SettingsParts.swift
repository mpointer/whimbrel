import AppKit
import SwiftUI

// The settings parts, first half: what every row shares (focus ring, reflow, row frame), the drawn controls
// (checkbox, segmented, stepper, menu, field), status chips, notice boxes, and the card.
//
// Rules the parts follow, so the app does not have to repeat them:
// - Every part is a plain view. An outside modifier (`.id`, `.settingsAnchor`, `.accessibilityIdentifier`) works on a
//   card, a row or a list, and a `ScrollViewReader` can scroll to it. Rows take `highlighted:` for the brief "here it is"
//   flash after a search jump; the app owns the timer.
// - A row that is not available says why in a line under its description, and the control is dimmed. Dimming alone is never the message.
// - Rows reflow: when the label would be squeezed (a narrow window or large text) the control drops under the label.

// MARK: Environment

extension EnvironmentValues {
    /// Draws the focus ring even without keyboard focus. The gallery sets it; an offscreen render has no focus.
    @Entry var wForceFocusRing: Bool = false
    /// True inside a `WSettingsCard`: rows draw the hairline that separates them from what is above.
    @Entry var wInSettingsCard: Bool = false
    /// The width every "Keep forever" style companion in a card takes, so the steppers beside them sit at one x. Nil: natural width.
    @Entry var wCompanionWidth: CGFloat? = nil
    /// The same for the stepper itself, so "30 days" and "365 days" rows still line up.
    @Entry var wStepperWidth: CGFloat? = nil
}

public extension EnvironmentValues {
    /// Changes each time the Settings window closes. A secret row that sees it drops what was typed and not saved; nothing else
    /// reacts, so the rest of the window keeps its state.
    @Entry var wSecretClearToken: UUID = UUID()
}

// MARK: Focus ring

private struct WFocusRingModifier: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.wForceFocusRing) private var forced
    @FocusState private var focused: Bool
    let radius: CGFloat?

    func body(content: Content) -> some View {
        content
            .focused($focused)
            .overlay {
                if focused || forced {
                    RoundedRectangle(cornerRadius: (radius ?? theme.shape.controlRadius) + 2, style: .continuous)
                        .strokeBorder(theme.focus, lineWidth: 2.5)
                        .padding(-2)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
    }
}

extension View {
    /// A visible keyboard focus ring in the `focus` token, 2.5 points wide.
    func wFocusRing(radius: CGFloat? = nil) -> some View {
        modifier(WFocusRingModifier(radius: radius))
    }
}

// MARK: Hairline

struct WHairline: View {
    @Environment(\.wTheme) private var theme
    var body: some View {
        Rectangle()
            .fill(theme.border)
            .frame(height: max(theme.borderWidth, 0.5))
            .accessibilityHidden(true)
    }
}

// MARK: Reflow

/// A label block and a control. Side by side while the label keeps `minLeading` points; otherwise the control
/// drops under the label. Used by every row, so all rows reflow the same way.
struct WReflow: Layout {
    var spacing: CGFloat
    var stackSpacing: CGFloat
    var minLeading: CGFloat
    /// When stacked, the control takes the whole width (a field or a segmented control).
    var stackedControlFills: Bool = false
    /// Side by side, top-align the control with the label's first line instead of centring it.
    var alignsTop: Bool = false
    /// Side by side, the label sits against the control's first line (of this height) instead of the middle of the whole control,
    /// for a control that carries a note under its first line. A label taller than the line stays at the top.
    var firstLineHeight: CGFloat? = nil

    struct Plan {
        var stacked: Bool
        var label: CGSize
        var control: CGSize
        var size: CGSize
    }

    private func plan(width: CGFloat?, subviews: Subviews) -> Plan {
        guard let label = subviews.first else { return Plan(stacked: false, label: .zero, control: .zero, size: .zero) }
        guard subviews.count > 1 else {
            let l = label.sizeThatFits(ProposedViewSize(width: width, height: nil))
            return Plan(stacked: false, label: l, control: .zero, size: l)
        }
        let control = subviews[1]
        let controlIdeal = control.sizeThatFits(.unspecified)
        let labelIdeal = label.sizeThatFits(.unspecified)
        guard let width, width.isFinite else {
            let size = CGSize(width: labelIdeal.width + spacing + controlIdeal.width, height: max(labelIdeal.height, controlIdeal.height))
            return Plan(stacked: false, label: labelIdeal, control: controlIdeal, size: size)
        }
        let labelMin = min(minLeading, labelIdeal.width)
        if width >= labelMin + spacing + controlIdeal.width {
            let l = label.sizeThatFits(ProposedViewSize(width: width - spacing - controlIdeal.width, height: nil))
            return Plan(stacked: false, label: l, control: controlIdeal, size: CGSize(width: width, height: max(l.height, controlIdeal.height)))
        }
        let l = label.sizeThatFits(ProposedViewSize(width: width, height: nil))
        let c = control.sizeThatFits(ProposedViewSize(width: width, height: nil))
        let cw = stackedControlFills ? width : min(c.width, width)
        let ch = stackedControlFills ? control.sizeThatFits(ProposedViewSize(width: width, height: nil)).height : c.height
        return Plan(stacked: true, label: l, control: CGSize(width: cw, height: ch), size: CGSize(width: width, height: l.height + stackSpacing + ch))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        plan(width: proposal.width, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let p = plan(width: bounds.width, subviews: subviews)
        guard let label = subviews.first else { return }
        var labelY = (p.stacked || alignsTop) ? bounds.minY : bounds.minY + (p.size.height - p.label.height) / 2
        if !p.stacked, !alignsTop, let line = firstLineHeight { labelY = bounds.minY + max(0, (line - p.label.height) / 2) }
        label.place(at: CGPoint(x: bounds.minX, y: labelY), anchor: .topLeading, proposal: ProposedViewSize(width: p.label.width, height: p.label.height))
        guard subviews.count > 1 else { return }
        let control = subviews[1]
        if p.stacked {
            control.place(at: CGPoint(x: bounds.minX, y: bounds.minY + p.label.height + stackSpacing), anchor: .topLeading, proposal: ProposedViewSize(width: p.control.width, height: p.control.height))
        } else {
            let y = alignsTop ? bounds.minY : bounds.minY + (p.size.height - p.control.height) / 2
            control.place(at: CGPoint(x: bounds.maxX - p.control.width, y: y), anchor: .topLeading, proposal: ProposedViewSize(width: p.control.width, height: p.control.height))
        }
    }
}

// MARK: Row frame

/// The shared look of every settings row: label, description, an unavailable reason, a control, a hairline above
/// (inside a card), a highlight, and a 44 point minimum height.
struct WSettingsRowFrame<Control: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wInSettingsCard) private var inCard
    @Environment(\.wStrings) private var strings
    let label: String
    let detail: String?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    /// The control carries the row's label and description for VoiceOver, so the visible text is not read twice.
    let controlSpeaksLabel: Bool
    let controlFills: Bool
    /// The control sits before the label (a checkbox).
    let leading: Bool
    /// The control always sits under the label and takes the full width (a secret, a list, a progress bar).
    let alwaysStacked: Bool
    /// An error line under the label block.
    let errorText: String?
    /// The label's colour, when it is not the primary ink (a link).
    let labelColor: Color?
    /// Height of the control's first line, when the control has a note under it: the label lines up with that line.
    let controlFirstLine: CGFloat?
    let control: Control

    init(
        label: String, detail: String?, isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false,
        controlSpeaksLabel: Bool = true, controlFills: Bool = false, leading: Bool = false, alwaysStacked: Bool = false,
        errorText: String? = nil, labelColor: Color? = nil, controlFirstLine: CGFloat? = nil, @ViewBuilder control: () -> Control
    ) {
        self.controlFirstLine = controlFirstLine
        self.alwaysStacked = alwaysStacked
        self.errorText = errorText
        self.labelColor = labelColor
        self.label = label
        self.detail = detail
        self.isEnabled = isEnabled
        self.disabledReason = isEnabled ? nil : disabledReason
        self.highlighted = highlighted
        self.controlSpeaksLabel = controlSpeaksLabel
        self.controlFills = controlFills
        self.leading = leading
        self.control = control()
    }

    private var spokenHint: String {
        [detail, disabledReason.map(strings.settings.unavailable)].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
    }

    private var labelBlock: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            if !label.isEmpty {
                Text(label).wFont(.body).foregroundStyle(labelColor ?? theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            }
            if let detail, !detail.isEmpty {
                Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            if let disabledReason, !disabledReason.isEmpty {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(5)) {
                    WGlyph(name: "info.circle", size: 11, weight: .semibold)
                    Text(disabledReason).fixedSize(horizontal: false, vertical: true)
                }
                .wFont(.secondary)
                .foregroundStyle(theme.textSecondary)
                .padding(.top, theme.space(2))
            }
            if let errorText, !errorText.isEmpty {
                WErrorLine(text: errorText).padding(.top, theme.space(2))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityHidden(controlSpeaksLabel)
    }

    @ViewBuilder
    private var inner: some View {
        if leading {
            HStack(alignment: .top, spacing: theme.space(10)) {
                control
                    .disabled(!isEnabled)
                    .accessibilityLabel(controlSpeaksLabel ? label : "")
                    .accessibilityHint(controlSpeaksLabel ? spokenHint : "")
                labelBlock
            }
        } else if alwaysStacked {
            VStack(alignment: .leading, spacing: theme.space(8)) {
                labelBlock
                control
                    .disabled(!isEnabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            WReflow(spacing: theme.space(16), stackSpacing: theme.space(8), minLeading: 200 * k, stackedControlFills: controlFills, firstLineHeight: controlFirstLine) {
                labelBlock
                control
                    .disabled(!isEnabled)
                    .accessibilityHint(controlSpeaksLabel ? spokenHint : "")
            }
        }
    }

    var body: some View {
        inner
            .padding(.horizontal, theme.space(14))
            .padding(.vertical, theme.space(10))
            .frame(minHeight: 44 * k)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(highlighted ? theme.highlight : .clear)
            .overlay(alignment: .leading) {
                if highlighted {
                    Rectangle().fill(theme.accentPrimary).frame(width: 3 * k)
                        .accessibilityHidden(true)
                }
            }
            .overlay(alignment: .top) {
                if inCard { WHairline() }
            }
            .overlay {
                if highlighted && theme.isHighContrast {
                    Rectangle().strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth).allowsHitTesting(false)
                }
            }
            .accessibilityElement(children: .contain)
    }
}

// MARK: Disabled look for drawn controls

extension View {
    func wDisabledLook() -> some View { modifier(WDisabledLook()) }
}

/// A button with no styling of its own. `.plain` also dims a disabled button, on top of `wDisabledLook`, which left a disabled
/// switch or stepper nearly invisible; this one leaves the dimming to `wDisabledLook` alone.
struct WFlatButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label }
}

private struct WDisabledLook: ViewModifier {
    @Environment(\.isEnabled) private var isEnabled
    func body(content: Content) -> some View { content.opacity(isEnabled ? 1 : 0.7) }
}

// MARK: Drawn controls

/// How a toggle row draws its control.
public enum WToggleStyle: Sendable { case `switch`, checkbox }

/// A checkbox, drawn so it follows the text scale. With a label it is the whole hit target. VoiceOver reads it as a checkbox.
public struct WCheckbox: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let showsLabel: Bool
    let isOn: Bool
    let onChange: (Bool) -> Void

    public init(_ label: String, isOn: Bool, showsLabel: Bool = true, onChange: @escaping (Bool) -> Void) {
        self.label = label
        self.isOn = isOn
        self.showsLabel = showsLabel
        self.onChange = onChange
    }

    public var body: some View {
        let side = 18 * k
        let box = RoundedRectangle(cornerRadius: max(3, theme.shape.controlRadius * 0.5), style: .continuous)
        Button { onChange(!isOn) } label: {
            HStack(spacing: theme.space(8)) {
                ZStack {
                    box.fill(isOn ? theme.accentPrimary : theme.backgroundCard)
                    box.strokeBorder(isOn ? theme.accentPrimary : (theme.isHighContrast ? theme.textPrimary : theme.firmEdge), lineWidth: theme.isHighContrast ? 2 : 1.25)
                    if isOn { WGlyph(name: "checkmark", size: 11, weight: .bold).foregroundStyle(theme.textOnAccent) }
                }
                .frame(width: side, height: side)
                if showsLabel { Text(label).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true) }
            }
            .frame(minHeight: WTokens.Size.target * k)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .wFocusRing(radius: 4)
        .wDisabledLook()
        .accessibilityRepresentation {
            Toggle(label, isOn: Binding(get: { isOn }, set: { onChange($0) })).toggleStyle(.checkbox)
        }
    }
}

/// A switch, drawn so it follows the text scale and looks the same whether or not the window is active. On is the
/// accent track, the knob at the trailing end and a check in the knob; off is an outlined track with the knob at the start.
struct WSwitch: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let label: String
    let isOn: Bool
    let onChange: (Bool) -> Void

    var body: some View {
        let w = 40 * k, h = 24 * k, pad = 3 * k
        let knob = h - pad * 2
        Button {
            onChange(!isOn)
        } label: {
            ZStack(alignment: isOn ? .trailing : .leading) {
                Capsule().fill(isOn ? theme.accentPrimary : theme.backgroundCard)
                RoundedRectangle(cornerRadius: 99, style: .continuous).wBorder(isOn ? theme.accentPrimary : (theme.isHighContrast ? theme.textPrimary : theme.firmEdge), style: StrokeStyle(lineWidth: theme.isHighContrast ? 2 : 1.25))
                Circle()
                    .fill(isOn ? theme.textOnAccent : (theme.isHighContrast ? theme.textPrimary : theme.textSecondary))
                    .frame(width: knob, height: knob)
                    .overlay {
                        if isOn { WGlyph(name: "checkmark", size: 9, weight: .bold).foregroundStyle(theme.accentPrimary) }
                    }
                    .padding(pad)
            }
            .frame(width: w, height: h)
            .animation(reduceMotion ? nil : .easeInOut(duration: WTokens.Motion.fast), value: isOn)
            .frame(minHeight: WTokens.Size.target * k)
            .contentShape(Rectangle())
        }
        .buttonStyle(WFlatButtonStyle())
        .wFocusRing(radius: 99)
        .wDisabledLook()
        .accessibilityRepresentation {
            Toggle(label, isOn: Binding(get: { isOn }, set: { onChange($0) })).toggleStyle(.switch)
        }
    }
}

/// Children of equal width: the widest one's ideal width, or the proposed width shared out when it is smaller (the text then wraps).
struct WEqualWidthLayout: Layout {
    var spacing: CGFloat

    private func widths(_ proposal: ProposedViewSize, _ subviews: Subviews) -> (each: CGFloat, count: Int) {
        let ideal = subviews.map { $0.sizeThatFits(.unspecified).width }.max() ?? 0
        let n = subviews.count
        guard n > 0 else { return (0, 0) }
        if let w = proposal.width, w.isFinite {
            let share = (w - spacing * CGFloat(n - 1)) / CGFloat(n)
            return (min(ideal, max(share, 1)), n)
        }
        return (ideal, n)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let (each, n) = widths(proposal, subviews)
        guard n > 0 else { return .zero }
        let height = subviews.map { $0.sizeThatFits(ProposedViewSize(width: each, height: nil)).height }.max() ?? 0
        return CGSize(width: each * CGFloat(n) + spacing * CGFloat(n - 1), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (each, n) = widths(ProposedViewSize(width: bounds.width, height: nil), subviews)
        guard n > 0 else { return }
        let height = bounds.height
        for (i, subview) in subviews.enumerated() {
            subview.place(at: CGPoint(x: bounds.minX + CGFloat(i) * (each + spacing), y: bounds.minY), anchor: .topLeading, proposal: ProposedViewSize(width: each, height: height))
        }
    }
}

struct WSegmentOption<ID: Hashable>: Identifiable {
    let id: ID
    let title: String
}

/// A segmented control drawn with the theme. One Tab stop; Left and Right change the choice. Selected: filled, outlined, bold.
struct WSegmentedControl<ID: Hashable>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var focused: Bool
    @Environment(\.wForceFocusRing) private var forced
    let label: String
    let options: [WSegmentOption<ID>]
    let selection: ID?
    let onSelect: (ID) -> Void

    private var index: Int? { options.firstIndex { $0.id == selection } }

    private func move(_ key: WListKey) {
        guard let next = WListNavigation.index(after: index, count: options.count, key: key), next != index else { return }
        onSelect(options[next].id)
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        WEqualWidthLayout(spacing: 2) {
            ForEach(options) { option in
                let on = option.id == selection
                let seg = RoundedRectangle(cornerRadius: max(2, theme.shape.controlRadius - 2), style: .continuous)
                Text(option.title)
                    .wFont(on ? .bodyStrong : .body)
                    .foregroundStyle(theme.textPrimary)
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .padding(.horizontal, theme.space(12))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .frame(minHeight: WTokens.Size.target * k)
                    .background(seg.fill(on ? theme.backgroundCard : .clear))
                    .overlay {
                        if on { seg.strokeBorder(theme.isHighContrast ? theme.textPrimary : theme.accentPrimary, lineWidth: theme.isHighContrast ? 2 : 1.5) }
                    }
                    .contentShape(seg)
                    .onTapGesture { onSelect(option.id) }
            }
        }
        .padding(2)
        .background(shape.fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.4)))
        .overlay(shape.strokeBorder(theme.isHighContrast ? theme.textPrimary : theme.controlBorder, lineWidth: theme.isHighContrast ? 1.5 : 0.75))
        .overlay {
            if focused || forced {
                shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-2).allowsHitTesting(false).accessibilityHidden(true)
            }
        }
        .focusable()
        .focused($focused)
        .onKeyPress(.leftArrow) { move(.previous); return .handled }
        .onKeyPress(.rightArrow) { move(.next); return .handled }
        .onKeyPress(.home) { move(.first); return .handled }
        .onKeyPress(.end) { move(.last); return .handled }
        .wDisabledLook()
        .accessibilityRepresentation {
            Picker(label, selection: Binding(get: { selection ?? options.first?.id ?? (options[0].id) }, set: { onSelect($0) })) {
                ForEach(options) { Text($0.title).tag($0.id) }
            }
            .pickerStyle(.segmented)
        }
    }
}

/// A menu picker drawn with the theme: the choice, then a chevron. Opens the system menu.
struct WMenuControl<ID: Hashable>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let options: [WSegmentOption<ID>]
    let selection: ID?
    let placeholder: String
    let onSelect: (ID) -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        let current = options.first { $0.id == selection }?.title
        Menu {
            ForEach(options) { option in
                Button { onSelect(option.id) } label: {
                    if option.id == selection { Label(option.title, systemImage: "checkmark") } else { Text(option.title) }
                }
            }
        } label: {
            HStack(spacing: theme.space(8)) {
                Text(current ?? placeholder).wFont(.body).foregroundStyle(current == nil ? theme.textSecondary : theme.textPrimary).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: theme.space(4))
                WGlyph(name: "chevron.up.chevron.down", size: 10, weight: .semibold).foregroundStyle(theme.textSecondary)
            }
            .padding(.horizontal, theme.space(10))
            .frame(minWidth: 140 * k, maxWidth: 280 * k, minHeight: WTokens.Size.target * k)
            .background(shape.fill(theme.backgroundCard))
            .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
            .contentShape(shape)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .wFocusRing()
        .wDisabledLook()
        .accessibilityLabel(label)
        .accessibilityValue(current ?? placeholder)
    }
}

/// A number stepper drawn with the theme. The text is the value with its unit ("30 days"). One Tab stop; Up and Down step.
struct WStepperControl: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var focused: Bool
    @Environment(\.wForceFocusRing) private var forced
    @Environment(\.wStepperWidth) private var sharedWidth
    let label: String
    let stepper: WNumberStepper
    let text: String
    let onStep: (Int) -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        HStack(spacing: 0) {
            stepButton("minus", direction: -1, enabled: stepper.canDecrease, name: strings.settings.decrease)
            Text(text)
                .wFont(.body).monospacedDigit()
                .foregroundStyle(theme.textPrimary)
                .lineLimit(1).fixedSize()
                .padding(.horizontal, theme.space(10))
                .frame(minWidth: 72 * k, minHeight: WTokens.Size.target * k)
            stepButton("plus", direction: 1, enabled: stepper.canIncrease, name: strings.settings.increase)
        }
        // Its own width is reported, and a card that shares one gives every stepper the widest.
        .background(GeometryReader { Color.clear.preference(key: WStepperWidthKey.self, value: $0.size.width) })
        .frame(minWidth: sharedWidth)
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
        .overlay {
            if focused || forced {
                shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-2).allowsHitTesting(false).accessibilityHidden(true)
            }
        }
        .focusable()
        .focused($focused)
        .onKeyPress(.upArrow) { if let v = stepper.stepped(1) { onStep(v) }; return .handled }
        .onKeyPress(.downArrow) { if let v = stepper.stepped(-1) { onStep(v) }; return .handled }
        .wDisabledLook()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(text)
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: if let v = stepper.stepped(1) { onStep(v) }
            case .decrement: if let v = stepper.stepped(-1) { onStep(v) }
            @unknown default: break
            }
        }
    }

    private func stepButton(_ glyph: String, direction: Int, enabled: Bool, name: String) -> some View {
        Button {
            if let v = stepper.stepped(direction) { onStep(v) }
        } label: {
            WGlyph(name: glyph, size: 12, weight: .semibold)
                .foregroundStyle(enabled ? theme.textPrimary : theme.textSecondary)
                .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                .contentShape(Rectangle())
        }
        .buttonStyle(WFlatButtonStyle())
        .focusable(false)
        .disabled(!enabled)
        .accessibilityLabel(name)
    }
}

/// A text field drawn with the theme. No width cap: the caller sets the width. Shows an error edge and a focus ring.
struct WTextBox: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wForceFocusRing) private var forced
    @FocusState private var focused: Bool
    let label: String
    let placeholder: String
    let text: Binding<String>
    var isSecure = false
    var monospaced = false
    var hasError = false
    var onSubmit: (() -> Void)? = nil
    var onFocusChange: ((Bool) -> Void)? = nil

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        Group {
            if isSecure {
                SecureField(placeholder, text: text)
            } else {
                TextField(placeholder, text: text)
            }
        }
        .onSubmit { onSubmit?() }
        .textFieldStyle(.plain)
        .wFont(.body)
        .font(monospaced ? .system(size: WTextRole.body.size(scale: k), design: .monospaced) : nil)
        .foregroundStyle(theme.textPrimary)
        .focused($focused)
        .onChange(of: focused) { _, now in onFocusChange?(now) }
        .padding(.horizontal, theme.space(10))
        .frame(minHeight: WTokens.Size.target * k)
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(hasError ? theme.error : theme.controlBorder, style: theme.borderStroke.widened(hasError || theme.isHighContrast ? 2 : 1)))
        .overlay {
            if focused || forced { shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-1).allowsHitTesting(false).accessibilityHidden(true) }
        }
        .wDisabledLook()
        .accessibilityLabel(label)
    }
}

// MARK: Inline lines

/// An error under the control it belongs to. Announced when it appears.
public struct WErrorLine: View {
    @Environment(\.wTheme) private var theme
    let text: String
    public init(text: String) { self.text = text }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: theme.space(6)) {
            WGlyph(name: "exclamationmark.circle.fill", size: 12, weight: .semibold)
            Text(text).wFont(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(theme.error)
        .accessibilityElement(children: .combine)
        .onAppear { AccessibilityNotification.Announcement(text).post() }
    }
}

/// A positive result line: a check and the words.
public struct WSuccessLine: View {
    @Environment(\.wTheme) private var theme
    let text: String
    public init(text: String) { self.text = text }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: theme.space(6)) {
            WGlyph(name: "checkmark.circle.fill", size: 12, weight: .semibold)
            Text(text).wFont(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(theme.success)
        .accessibilityElement(children: .combine)
    }
}

/// A caution line: a triangle and the words.
public struct WWarningLine: View {
    @Environment(\.wTheme) private var theme
    let text: String
    public init(text: String) { self.text = text }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: theme.space(6)) {
            WGlyph(name: "exclamationmark.triangle.fill", size: 12, weight: .semibold)
            Text(text).wFont(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(theme.isDark ? theme.warning : theme.textPrimary)
        .accessibilityElement(children: .combine)
    }
}

/// A neutral status line under a row: "Saved. Checking the key…", "Last updated 10:42".
public struct WStatusText: View {
    @Environment(\.wTheme) private var theme
    let text: String
    public init(_ text: String) { self.text = text }
    public var body: some View {
        Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: Status chip

public enum WStatusTone: Sendable, CaseIterable { case neutral, good, warning, problem }

/// A small state label ("Encrypted", "Needs attention", "Default", "Beta coming soon"). The glyph and the words carry the
/// state; colour only adds to them. Spoken with its tone.
public struct WStatusChip: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let text: String
    let tone: WStatusTone
    let glyph: String?

    public init(_ text: String, tone: WStatusTone = .neutral, glyph: String? = nil) {
        self.text = text
        self.tone = tone
        self.glyph = glyph
    }

    static func defaultGlyph(_ tone: WStatusTone) -> String? {
        switch tone {
        case .neutral: return nil
        case .good: return "checkmark.circle.fill"
        case .warning: return "exclamationmark.triangle.fill"
        case .problem: return "xmark.octagon.fill"
        }
    }

    private var toneColor: Color {
        switch tone {
        case .neutral: return theme.textSecondary
        case .good: return theme.success
        case .warning: return theme.isDark ? theme.warning : theme.textPrimary
        case .problem: return theme.error
        }
    }

    private var spokenTone: String? {
        switch tone {
        case .neutral: return nil
        case .good: return strings.settings.toneGood
        case .warning: return strings.settings.toneWarning
        case .problem: return strings.settings.toneProblem
        }
    }

    public var body: some View {
        let edge: Color = tone == .neutral ? theme.firmEdge : (tone == .warning && !theme.isDark ? theme.textSecondary : toneColor)
        HStack(spacing: theme.space(5)) {
            // A neutral pill has no icon of its own: an empty circle read as an unselected radio button.
            if let name = glyph ?? Self.defaultGlyph(tone) { WGlyph(name: name, size: 11, weight: .semibold).foregroundStyle(toneColor) }
            Text(text).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textPrimary).lineLimit(2).fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, theme.space(9))
        .padding(.vertical, theme.space(3))
        .background(Capsule().fill(toneColor.opacity(theme.isHighContrast ? 0 : 0.12)))
        .overlay(RoundedRectangle(cornerRadius: 99, style: .continuous).wBorder(edge, style: StrokeStyle(lineWidth: theme.isHighContrast ? 1.5 : 1)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel([spokenTone, text].compactMap { $0 }.joined(separator: ": "))
    }
}

// MARK: Notice box

public enum WNoticeTone: Sendable, CaseIterable { case info, warning, problem }

public struct WNoticeAction: Identifiable {
    public let id: String
    public let title: String
    public let kind: WButtonKind
    public let run: () -> Void
    public init(id: String? = nil, _ title: String, kind: WButtonKind = .secondary, run: @escaping () -> Void) {
        self.id = id ?? title
        self.title = title
        self.kind = kind
        self.run = run
    }
}

/// A bordered box for a notice that matters: a warning before a deletion, a plan notice, a failed setup. Glyph, optional title,
/// text and optional buttons. Short or quiet text is better as a description under a row.
public struct WNoticeBox: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let tone: WNoticeTone
    let title: String?
    let text: String
    let actions: [WNoticeAction]

    public init(tone: WNoticeTone = .info, title: String? = nil, text: String, actions: [WNoticeAction] = []) {
        self.tone = tone
        self.title = title
        self.text = text
        self.actions = actions
    }

    private var glyph: String {
        switch tone {
        case .info: return "info.circle.fill"
        case .warning: return "exclamationmark.triangle.fill"
        case .problem: return "exclamationmark.octagon.fill"
        }
    }

    private var toneColor: Color {
        switch tone {
        case .info: return theme.accentPrimary
        case .warning: return theme.isDark ? theme.warning : theme.textPrimary
        case .problem: return theme.error
        }
    }

    private var spokenTone: String {
        switch tone {
        case .info: return strings.settings.noticeInfo
        case .warning: return strings.settings.noticeWarning
        case .problem: return strings.settings.noticeProblem
        }
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        let edge: Color = tone == .warning && !theme.isDark ? theme.textSecondary : toneColor
        HStack(alignment: .top, spacing: theme.space(10)) {
            WGlyph(name: glyph, size: 15, weight: .semibold).foregroundStyle(toneColor).padding(.top, 1)
            VStack(alignment: .leading, spacing: theme.space(6)) {
                if let title, !title.isEmpty {
                    Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                }
                Text(text).wFont(.secondary).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                if !actions.isEmpty {
                    WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                        ForEach(actions) { action in WButton(action.title, kind: action.kind, action: action.run) }
                    }
                    .padding(.top, theme.space(2))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(theme.space(12))
        .background(shape.fill(toneColor.opacity(theme.isHighContrast ? 0 : 0.08)))
        .overlay(shape.wBorder(edge, style: theme.borderStroke.widened(theme.isHighContrast ? 2 : 1)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(spokenTone)
    }
}

// MARK: Card

/// A settings card: a heading, an optional description and status chip, then rows separated by hairlines.
/// The rows are plain views; put anything in `content`. Apply the app's anchor modifier to the card or to a row.
public struct WSettingsCard<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let heading: String?
    let detail: String?
    let statusText: String?
    let statusTone: WStatusTone
    let content: Content

    public init(
        heading: String? = nil, description: String? = nil, statusText: String? = nil, statusTone: WStatusTone = .neutral,
        @ViewBuilder content: () -> Content
    ) {
        self.heading = heading
        self.detail = description
        self.statusText = statusText
        self.statusTone = statusTone
        self.content = content()
    }

    private var hasHeader: Bool { heading != nil || detail != nil || statusText != nil }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            if hasHeader {
                WReflow(spacing: theme.space(12), stackSpacing: theme.space(8), minLeading: 200 * k) {
                    VStack(alignment: .leading, spacing: theme.space(3)) {
                        if let heading {
                            Text(heading).wFont(.subtitle).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                                .accessibilityAddTraits(.isHeader)
                        }
                        if let detail {
                            Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if let statusText { WStatusChip(statusText, tone: statusTone) }
                }
                .padding(.horizontal, theme.space(14))
                .padding(.vertical, theme.space(12))
            }
            content
                .environment(\.wInSettingsCard, true)
                // With no header the first row's hairline would sit at the card's top edge. Slide it under the border.
                .padding(.top, hasHeader ? 0 : -1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(theme.backgroundCard))
        .clipShape(shape)
        .overlay(shape.wBorder(theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(heading ?? "")
    }
}
