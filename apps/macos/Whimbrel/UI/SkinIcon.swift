import SwiftUI

extension Image {
    /// The selected skin's SF Symbol for a role (sidebar tabs, Ask send, empty states).
    init(skinRole role: SkinIconRole) {
        self.init(systemName: SkinStore.shared.selected.icons[role])
    }
}
