import SwiftUI
import WhimbrelUI

/// `SyncWaitingView` in the redesign: the same `SyncState` calls and the same fresh start steps.
struct SyncWaitingRedesignView: View {
    @ObservedObject var syncState: SyncState
    private let presentationContextProvider = WindowPresentationContextProvider()
    @State private var showingFreshStart = false
    @State private var startingFresh = false

    var body: some View {
        WSyncWaitingView(model: content).redesignThemed()
    }

    private var content: WSyncWaitingModel {
        var m = WSyncWaitingModel()
        switch syncState.status {
        case .waitingForGroupKey:
            m.title = String(localized: "Waiting for a device to finish setting this one up")
            m.message = String(localized: "This device was approved, but the handoff to send it the sync key expired before it could be delivered. Open Whimbrel on an already-signed-in device, go to Settings → Devices, and approve this device again.")
            m.checkAgainTitle = String(localized: "Check again", comment: "Button that checks again whether another device has finished setting this one up.")
        default:
            m.title = String(localized: "Waiting for approval on another device")
            if let holder = syncState.swapHolderName {
                m.message = SyncState.swapWaitingMessage(holderName: holder)
            } else {
                m.message = String(localized: "Open Whimbrel on a device that's already signed in, go to Settings → Devices, and approve this one. It will be picked up automatically within a few seconds.", comment: "Shown while this device waits for another device to approve it.")
            }
        }
        if syncState.status == .waitingForApproval, let holder = syncState.swapHolderName {
            if showingFreshStart {
                m.freshStartWarning = SyncState.freshStartWarning(holderName: holder)
            } else {
                m.freshStartPrompt = String(localized: "\(SyncState.holderAtSentenceStart(holder)) isn't available?")
            }
            m.actionError = syncState.actionError
        }
        m.startFreshTitle = String(localized: "Start fresh")
        m.cancelTitle = String(localized: "Cancel")
        m.startingFresh = startingFresh
        m.onCheckAgain = { Task { await syncState.enableSync(presentationContextProvider: presentationContextProvider) } }
        m.onShowFreshStart = { showingFreshStart = true }
        m.onCancelFreshStart = { showingFreshStart = false }
        m.onStartFresh = {
            Task {
                startingFresh = true
                await syncState.startFresh()
                startingFresh = false
            }
        }
        return m
    }
}
