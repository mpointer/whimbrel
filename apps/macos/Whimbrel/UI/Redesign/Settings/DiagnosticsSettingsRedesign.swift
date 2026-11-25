import SwiftUI

/// Diagnostics in the redesigned Settings window. It is a developer surface that only Debug builds list, so it is the one
/// section that still hosts its classic pane, unchanged, inside a card. It is not redrawn.
struct DiagnosticsSettingsRedesignView: View {
    var body: some View {
        ClassicPaneCard { DiagnosticsSettingsPane() }
        #if DEBUG
        ClassicPaneCard { DesignSystemGalleryLauncher() }
        #endif
    }
}
