#if DEBUG
import SwiftUI
import WhimbrelUI

/// Debug builds only. Hosts the WhimbrelUI gallery so the new components can be checked in the app.
struct DesignSystemGalleryView: View {
    @State private var mode: WGalleryMode = .light
    @State private var textScale: Double = 1

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 16) {
                Picker("Mode", selection: $mode) {
                    ForEach(WGalleryMode.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 520)
                Picker("Text size", selection: $textScale) {
                    Text("100%").tag(1.0)
                    Text("150%").tag(1.5)
                    Text("200%").tag(2.0)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 220)
                Spacer()
            }
            .padding(12)
            Divider()
            WGallery(mode: mode, textScale: CGFloat(textScale))
        }
        .frame(minWidth: 900, minHeight: 600)
    }
}

/// A Diagnostics row that opens the gallery in a sheet.
struct DesignSystemGalleryLauncher: View {
    @State private var showing = false

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("Design system gallery")
                Text("Debug builds only.").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Open") { showing = true }
        }
        .sheet(isPresented: $showing) {
            VStack(spacing: 0) {
                DesignSystemGalleryView()
                Divider()
                HStack { Spacer(); Button("Close") { showing = false }.keyboardShortcut(.cancelAction) }.padding(12)
            }
            .frame(width: 1100, height: 760)
        }
    }
}
#endif
