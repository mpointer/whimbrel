import SwiftUI
import WhimbrelUI

/// `LaunchStartingView` in the redesign: the page color, then a quiet bar and the same line after a second.
struct LaunchStartingRedesignView: View {
    var body: some View {
        WLaunchStartingView(message: EncryptionCopy.starting, delay: .seconds(1)).redesignThemed()
    }
}
