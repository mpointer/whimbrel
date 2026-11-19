import SwiftUI
import WhimbrelUI

/// `LaunchMigrationView` in the redesign: the same words and a determinate bar, no Quit.
struct LaunchMigrationRedesignView: View {
    let fraction: Double

    var body: some View {
        WNoticeScreenView(model: Self.model(fraction: fraction)).redesignThemed()
    }

    static func model(fraction: Double) -> WNoticeScreenModel {
        var m = WNoticeScreenModel()
        m.title = EncryptionCopy.migratingTitle
        m.paragraphs = [EncryptionCopy.migratingBody]
        m.hints = [EncryptionCopy.migratingInterruptHint]
        m.fraction = min(max(fraction, 0), 1)
        return m
    }
}
