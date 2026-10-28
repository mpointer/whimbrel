import SwiftUI

/// FR-UI-2 follow-on: a real detail window for one screen-text capture, opened by
/// tapping its Timeline row (which only ever shows a 2-line-truncated preview) — mirrors
/// `MeetingDetailView`'s exact ID-based-fetch pattern (fresh data via `getCaptureItem`,
/// not a value passed in at open time, so the window degrades gracefully if the item's
/// gone by the time it opens rather than showing stale or garbage data).
struct CaptureDetailView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    let captureItemId: Int64

    @State private var loadedItem: CoreCaptureItem?
    @State private var loadFailed = false

    var body: some View {
        Group {
            if let loadedItem {
                CaptureDetailContentView(item: loadedItem)
            } else if loadFailed {
                Text("This item is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 480, idealWidth: 560, minHeight: 400, idealHeight: 560)
        .navigationTitle(loadedItem?.windowTitle ?? "Capture")
        .background(Color.whimbrelBackgroundPage)
        .overlay(alignment: .topTrailing) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .accessibilityLabel("Close")
                    .font(.system(size: 20))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            .buttonStyle(.plain)
            .padding(Spacing.md)
            .help("Close")
        }
        .onAppear { loadItemIfNeeded() }
    }

    private func loadItemIfNeeded() {
        guard loadedItem == nil, !loadFailed else { return }
        guard let store = appState.coreStore else {
            loadFailed = true
            return
        }
        let id = captureItemId
        CoreMemoryStore.accessQueue.async {
            let result = store.getCaptureItem(id: id)
            DispatchQueue.main.async {
                if let result {
                    loadedItem = result
                } else {
                    loadFailed = true
                }
            }
        }
    }
}

private struct CaptureDetailContentView: View {
    let item: CoreCaptureItem

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(item.windowTitle)
                    .font(.whimbrelDisplay())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                HStack {
                    Text(item.appBundleId)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    if let source = item.source {
                        Text("· \(source == "ocr" ? "OCR fallback" : "Accessibility API")")
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    DeviceChip(deviceId: item.deviceId)
                    Spacer()
                    Text(Self.formatted(item.tsUnixMs))
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                if let url = item.url {
                    Text(url)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelLink)
                        .textSelection(.enabled)
                }
                Divider().overlay(Color.whimbrelBorder)
                Text(item.text)
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .textSelection(.enabled)
            }
            .padding(Spacing.xl)
        }
    }

    private static func formatted(_ tsUnixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(tsUnixMs) / 1000)
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}
