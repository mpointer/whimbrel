import SwiftUI

// Automations drawn on the settings parts: which device runs routines, then one card per routine with its switch, its
// timing, its model and, where the routine can run in the cloud, the cloud switch, the ready-by time and how fresh the
// last result is. The app builds the model and passes the actions.

public struct WAutomationsSettingsView: View {
    @Environment(\.wTheme) private var theme
    let model: WAutomationsModel
    let actions: WAutomationsActions
    let highlighted: String?

    public init(model: WAutomationsModel, actions: WAutomationsActions, highlighted: String? = nil) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
    }

    public var body: some View {
        let l = model.labels
        VStack(alignment: .leading, spacing: theme.space(16)) {
            WSettingsCard(heading: l.heading, description: l.intro) {
                WSettingsPickerRow<String>(
                    label: l.runsOn, description: model.runsOn.chosenLine, options: model.runsOn.options.map { WSettingsOption(id: $0.id, title: $0.title) },
                    selection: model.runsOn.selection, style: .menu, onSelect: actions.setRunsOn)
                if let note = model.runsOn.note { wNote(note) }
            }
            if model.routines.isEmpty {
                WSettingsCard { wNote(l.noRoutines) }
            }
            ForEach(model.routines) { routine in routineCard(routine) }
        }
    }

    private func routineCard(_ routine: WRoutineRow) -> some View {
        let l = model.labels
        return WSettingsCard {
            WSettingsRow(label: routine.name, isOn: routine.enabled, onChange: { actions.setEnabled(routine.id, $0) })
            if let autoApprove = routine.autoApprove {
                WSettingsRow(label: l.autoApprove, isOn: autoApprove, onChange: { actions.setAutoApprove(routine.id, $0) })
            }
            switch routine.timing {
            case .leadMinutes(let minutes, let range):
                WSettingsNumberRow(
                    label: l.prepareLeadNote(minutes), value: minutes, range: range, step: 5, unit: l.minutes,
                    onChange: actions.setLeadMinutes)
            case .dailyAt(let time):
                WTimeFieldRow(
                    label: l.runsDailyAt, placeholder: l.timePlaceholder, invalid: l.timeInvalid, stored: time,
                    onCommit: { actions.setSchedule(routine.id, $0) })
            }
            if let options = routine.modelOptions {
                WSettingsPickerRow<String>(
                    label: l.model, options: options.map { WSettingsOption(id: $0.id, title: $0.title) }, selection: routine.modelSelection,
                    style: .menu, onSelect: { actions.setModel(routine.id, $0) })
            }
            if let cloud = routine.cloud { cloudRows(routine, cloud) }
        }
    }

    @ViewBuilder
    private func cloudRows(_ routine: WRoutineRow, _ cloud: WRoutineCloud) -> some View {
        let l = model.labels
        let blocked = !cloud.isOn && cloud.blockedReason != nil
        WSettingsRow(
            label: l.cloudToggle, description: cloud.toggleCopy, isOn: cloud.isOn, isEnabled: !blocked, disabledReason: blocked ? cloud.blockedReason : nil,
            onChange: { actions.setCloud(routine.id, $0) })
        wNote(cloud.disclosure)
        if let note = cloud.statusNote { wNote(note) }
        if cloud.isOn {
            if let readyBy = cloud.readyBy {
                WReadyByRow(label: l.readyBy, note: cloud.readyByNote, time: readyBy, onChange: { actions.setReadyBy(routine.id, $0) })
            }
            if let freshness = cloud.freshness { wNote(freshness) }
        }
    }
}

/// A time of day typed as HH:MM. It writes only a valid time, on Return, when the field loses focus or as soon as five
/// characters make a valid time; anything else shows a message and writes nothing.
struct WTimeFieldRow: View {
    @Environment(\.wTextScale) private var k
    @State private var draft: String
    @State private var showError = false
    let label: String
    let placeholder: String
    let invalid: String
    let stored: String
    let onCommit: (String) -> Void

    init(label: String, placeholder: String, invalid: String, stored: String, initialText: String? = nil, onCommit: @escaping (String) -> Void) {
        _draft = State(initialValue: initialText ?? stored)
        _showError = State(initialValue: initialText != nil && WTimeRule.parse(initialText ?? "") == nil)
        self.label = label
        self.placeholder = placeholder
        self.invalid = invalid
        self.stored = stored
        self.onCommit = onCommit
    }

    private func commit() {
        if let time = WTimeRule.parse(draft) {
            showError = false
            if time != stored { onCommit(time) }
        } else {
            showError = true
        }
    }

    var body: some View {
        WSettingsRowFrame(label: label, detail: nil, controlSpeaksLabel: true, controlFills: false, errorText: showError ? invalid : nil) {
            WTextBox(
                label: label, placeholder: placeholder, text: $draft, hasError: showError, onSubmit: commit,
                onFocusChange: { focused in if !focused { commit() } })
                .frame(width: 100 * k)
        }
        .onChange(of: draft) { _, text in
            if text.count == 5, WTimeRule.parse(text) != nil { commit() } else if showError { showError = false }
        }
        .onChange(of: stored) { _, new in if WTimeRule.parse(draft) != nil { draft = new } }
        // The classic field wrote on every keystroke, so closing the window mid-edit lost nothing. A valid time is written on the way out.
        .onDisappear { if WTimeRule.parse(draft) != nil { commit() } }
    }
}

/// "Ready by": a time of day in the system's date field, labelled, in the window's colours.
struct WReadyByRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let note: String
    let time: Date
    let onChange: (Date) -> Void

    var body: some View {
        WSettingsRowFrame(label: label, detail: note, controlSpeaksLabel: false) {
            DatePicker(
                label, selection: Binding(get: { time }, set: onChange), displayedComponents: .hourAndMinute)
                .labelsHidden()
                .datePickerStyle(.field)
                .tint(theme.accentPrimary)
                .colorScheme(theme.isDark ? .dark : .light)
                .fixedSize()
                // The field style draws its own faint box around the time. Its edge is masked off, so the box below is the one
                // border, the same edge as the other time fields.
                .mask { Rectangle().padding(3) }
                // The system picker does not follow the window's text size, so it is scaled with it; the box around it is
                // sized as the other time fields are (100 wide, one control high, at the text size), not scaled as a picture.
                .scaleEffect(k)
                .frame(width: 100 * k, height: WTokens.Size.target * k)
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundCard))
                .overlay(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
                .frame(width: 150 * k, height: 44 * k, alignment: .trailing)
                .accessibilityLabel(label)
        }
    }
}
