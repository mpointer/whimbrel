import SwiftUI

/// Home under the skin's header band, as the other tabs are (`SkinHeaderArea`, minus the header image, which the
/// redesigned Home does not draw yet). A skin with no band gets `content` as it is, with no wrapper, so Home does not move.
struct TodayHomeBandArea<Content: View>: View {
    @ViewBuilder let content: () -> Content

    var body: some View {
        if SkinHeaderBandView.draws(SkinStore.shared.selected.decoration.headerBand) {
            VStack(spacing: 0) {
                SkinHeaderBandView()
                content()
            }
        } else {
            content()
        }
    }
}
