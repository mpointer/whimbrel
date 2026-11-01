import AppKit
import SwiftUI

/// Shown once per meeting when the recording notice is on.
struct RecordingNoticeCard: View {
    @ObservedObject var settings: RecordingNoticeSettings
    let meetingId: Int64
    let chat: ZoomChatPosting?
    @State private var delivery: RecordingNotice.Delivery?

    var body: some View {
        if settings.enabled, settings.dismissedMeetingId != meetingId {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(delivery == .posted ? "Notice posted in the chat." : "Let people know you're recording.")
                    .font(.whimbrelBody(.headline))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                if delivery != .posted {
                    Text(settings.text)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .textSelection(.enabled)
                }
                HStack {
                    if delivery != .posted {
                        Button("Copy") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(settings.text, forType: .string)
                        }
                        .buttonStyle(.whimbrelSecondary)
                    }
                    Button("Done") { settings.dismissedMeetingId = meetingId }
                        .buttonStyle(.whimbrelSecondary)
                }
            }
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .task(id: meetingId) {
                delivery = await RecordingNotice.deliver(settings.text, chat: chat)
            }
        }
    }
}
