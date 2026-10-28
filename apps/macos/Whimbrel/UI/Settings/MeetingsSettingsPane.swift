import SwiftUI
import AppKit

struct MeetingsSettingsPane: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Meetings").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)

            Toggle("Auto-start recording for calendar meetings", isOn: Binding(
                get: { appState.meetingTrigger.autoStartEnabled },
                set: { appState.meetingTrigger.autoStartEnabled = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)

            HStack {
                Text("Prompt lead time:")
                Stepper(
                    "\(Int(appState.meetingTrigger.leadMinutes)) min",
                    value: Binding(
                        get: { appState.meetingTrigger.leadMinutes },
                        set: { appState.meetingTrigger.leadMinutes = $0 }
                    ),
                    in: 1...10
                )
            }

            Text("A qualifying calendar meeting has a conferencing link (Zoom/Meet/Teams/Webex) or at least one other attendee. Upcoming meetings and the recording prompt itself are shown in the main window, not here.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Live transcript during meetings", isOn: Binding(
                get: { appState.liveTranscriptSettings.enabled },
                set: { appState.liveTranscriptSettings.enabled = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)

            Text("Transcribes while you record so you can read along and ask questions during the call. Uses extra CPU while recording and pauses in Low Power Mode. The full transcript is still made after the meeting and replaces the live one.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Picker("Ask if I'm still recording after", selection: Binding(
                get: { appState.recordingCheckInSettings.hours },
                set: { appState.recordingCheckInSettings.hours = $0 }
            )) {
                ForEach(RecordingCheckInSettings.allowedHours, id: \.self) { hours in
                    Text(hours == 0 ? "Off" : hours == 1 ? "1 hour" : "\(hours) hours").tag(hours)
                }
            }
            .fixedSize()

            Text("Shows a \"Still recording?\" prompt. With no answer in 5 minutes, the recording stops and is saved. Recordings always stop at 6 hours or when less than 2 GB is free. Applies from the next recording.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Detect meetings with no calendar entry", isOn: Binding(
                get: { appState.meetingTrigger.heuristicDetectionEnabled },
                set: { appState.meetingTrigger.heuristicDetectionEnabled = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)

            Text("Prompts when a known meeting app (Zoom, Teams, Slack, Webex, FaceTime) is running with the mic and system audio both active — for ad-hoc calls with nothing on your calendar. Uses the \"Auto-start\" setting above too. Doesn't cover a meeting joined from a browser tab (e.g. Meet) yet.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Away Mode", isOn: Binding(
                get: { appState.awayMode.enabled },
                set: { appState.awayMode.enabled = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)

            Text("While this is on, Whimbrel joins your Zoom meetings for you, muted with the camera off, records them, and posts a short note in the meeting chat. Keep this Mac unlocked and signed in to Zoom.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Text("In Zoom > Settings > Audio, turn on \"Automatically join audio by computer\" and \"Always mute my microphone when joining\". In Zoom > Settings > Video, turn on \"Turn off my video when joining\" and turn off \"Always show video preview dialog\".")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Text("Some places require everyone's consent before a meeting is recorded. The chat note tells people, but it isn't consent. Check the rules where you and the people you meet are.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Save slides and shared screens", isOn: Binding(
                get: { appState.snapshotSettings.enabled },
                set: { appState.snapshotSettings.enabled = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)

            Text("Saves one image per slide shown in recorded meetings to a folder you can browse. Needs Screen Recording permission."
                 + (appState.permissions.screenRecordingGranted ? "" : " Screen Recording permission is off."))
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Text("Snapshots folder")
                Text(appState.snapshotSettings.rootURL.path)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Button("Choose…") {
                    let panel = NSOpenPanel()
                    panel.canChooseFiles = false
                    panel.canChooseDirectories = true
                    panel.canCreateDirectories = true
                    panel.allowsMultipleSelection = false
                    if panel.runModal() == .OK, let url = panel.url {
                        appState.snapshotSettings.customRootPath = url.path
                    }
                }
                .buttonStyle(.whimbrelSecondary)
                .controlSize(.small)
                Button("Reset") { appState.snapshotSettings.customRootPath = nil }
                    .buttonStyle(.whimbrelSecondary)
                    .controlSize(.small)
                    .disabled(appState.snapshotSettings.customRootPath == nil)
            }

            Divider().overlay(Color.whimbrelBorder)

            Text("Always treat as meetings (#64)").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Some work Exchange/CalDAV accounts don't sync a conferencing link or attendee list at all, so those events never qualify above. Opt a calendar in here and every non-all-day event on it counts as a meeting.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            if appState.calendar.availableCalendars.isEmpty {
                Text("No calendars found — grant Calendar access first.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(appState.calendar.availableCalendars) { calendar in
                    Toggle(calendar.title, isOn: Binding(
                        get: { appState.calendar.alwaysQualifyCalendarIDs.contains(calendar.id) },
                        set: { isOn in
                            if isOn {
                                appState.calendar.alwaysQualifyCalendarIDs.insert(calendar.id)
                            } else {
                                appState.calendar.alwaysQualifyCalendarIDs.remove(calendar.id)
                            }
                        }
                    ))
                    .tint(Color.whimbrelAccentPrimary)
                }
            }

            Divider().overlay(Color.whimbrelBorder)

            HStack {
                Text("Transcript/summary reading size:")
                Stepper(
                    "\(Int(appState.readingFontSize)) pt",
                    value: $appState.readingFontSize,
                    in: 11...20
                )
            }
        }
    }
}
