import Foundation

// The English words of Sync & Devices, Connectors and Automations. The app fills every field from its string catalog; these
// are the defaults the gallery shows.

public struct WSyncLabels: Sendable {
    public var heading = "Devices"
    public var intro = "Sync your timeline across Macs. Sign in once per device, then approve new devices from here."
    public var statusLabel = "Sync"
    public var stateOff = "Off"
    public var stateSigningIn = "Signing in"
    public var stateWaiting = "Waiting for approval"
    public var stateOn = "On"
    public var stateLimit = "Device limit reached"
    public var stateFailed = "Sign-in failed"
    public var signingIn = "Signing in\u{2026}"
    public var enableSync = "Enable Sync"
    public var tryAgain = "Try Again"
    // Phone
    public var phoneHeading = "Whimbrel on your phone"
    public var phoneBlurb = "Read your meeting notes, decisions and action items on iPhone or Android. The apps are in beta."
    public var joinBeta = "Join the beta"
    public var scanWithPhone = "Scan with your phone"
    public var qrLabel = "QR code for the phone beta page"
    public var phoneConnected = "Phone: connected"
    // Devices
    public var deviceName = "This device's name"
    public var save = "Save"
    public var loadingDevices = "Loading devices\u{2026}"
    public var approve = "Approve"
    public var resendKey = "Re-send Key"
    public var revoke = "Revoke"
    public var remove = "Remove"
    public var cancel = "Cancel"
    public var codeLine: @Sendable (String) -> String = { "Check this matches the code on the new device: \($0)" }
    public var codeUnverifiable = "Can't verify this device"
    public var collisionTitle = "Another pending device shows the same code"
    public var collisionText = "Do not approve either one. Revoke the one you do not recognize."
    public var revokeTitle = "Revoke this device?"
    public var revokeMessage = "This device will lose access immediately. Every other device on this account will also need to silently reconnect. This cannot be undone from here. The device would need to sign in again and be approved again."
    public var removeRunnerTitle = "Remove Whimbrel Cloud?"
    public var removeRunnerMessage = ""
    public var addRunner = "Add Whimbrel Cloud\u{2026}"
    public var accountLabel = "Sync account"
    public var deleteAccount = "Delete my sync account\u{2026}"
    public var deleteAccountTitle = "Delete your sync account?"
    public var delete = "Delete"
    // Limit
    public var upgradePlus = "Upgrade to Plus for unlimited devices."
    public var upgradeCloud = "Upgrade to Cloud for unlimited devices."
    public var planReadFailed = "Couldn't check your plan. Try again in a moment."
    public var useThisComputer = "Use this computer instead"
    public var waitlistJoined = "You're on the waitlist. Thanks for telling us what you need."
    public var waitlistPrompt = "Want more? Tell us what you'd use:"
    public var joinWaitlist = "Join the waitlist"
    // Sync card
    public var syncHeading = "Sync"
    /// The name of the row whose button is "Sync now".
    public var syncRowLabel = "Sync"
    public var syncNow = "Sync now"
    public var syncing = "Syncing\u{2026}"
    public var removeThisDevice = "Remove this device\u{2026}"
    public var removeDeviceTitle = "Remove this device?"
    public var removeDeviceMessage = ""
    public var removeDeviceConfirm = "Remove"
    // Runner sheet
    public var runnerSheetTitle = "Whimbrel Cloud"
    public var runnerPublicKeyHelp = "On the host that will run your routines, run whimbrel-cloud-runner init and paste the public key it prints here."
    public var publicKey = "Public key"
    public var add = "Add"
    public var done = "Done"
    public var copy = "Copy"
    public var copied = "Copied"
    public var provisionInstructions = ""
    public var runnerNotShownAgain = "These values are not shown again."
    public var runnerCloseQuestion = "These values are not shown again. Close without copying the secret one?"
    public var closeWithoutCopying = "Close without copying"
    public var keepOpen = "Keep it open"
    public init() {}
}

public struct WConnectorsLabels: Sendable {
    public var heading = "Connectors"
    public var intro = "Connect external services so Whimbrel can use them. You can connect more than one account to each."
    public var notConnected = "Not connected."
    public var accounts = "Accounts"
    public var makeDefault = "Make Default"
    public var makeDefaultNamed: @Sendable (String) -> String = { "Make \($0) the default" }
    public var removeNamed: @Sendable (String) -> String = { "Remove \($0)" }
    public var defaultChip = "Default"
    public var remove = "Remove"
    public var connecting = "Connecting\u{2026}"
    public var advanced = "Advanced"
    public var ownClientHeading = "Use your own Google Cloud project"
    public var ownClientNote = "For Gmail reading. Create a Desktop OAuth client in your own Google Cloud project, add yourself as its user, and publish it (an unverified app is fine for yourself)."
    public var clientIdLabel = "Client ID"
    public var clientSecretLabel = "Client secret"
    public var ownClientSavedNote = ""
    public var removeClientTitle = "Remove your own Google client?"
    public var removeClientMessage = ""
    public var saveClientNote = ""
    // MCP
    public var mcpHeading = "MCP Clients"
    public var mcpIntro = "Let a local MCP client, such as an AI assistant, read your Whimbrel data through a scoped token you can revoke. Each token is shown once, right after you create it, so save it somewhere safe."
    public var serverToggle = "Enable MCP Server"
    public var addClientHeading = "New client"
    public var clientName = "Client name"
    public var clientNamePlaceholder = "For example Claude Desktop"
    public var permissions = "What it can use"
    public var addClient = "Add Client"
    public var noClients = "No MCP clients yet."
    public var clientsList = "Added clients"
    public var revoke = "Revoke"
    public var revokeNamed: @Sendable (String) -> String = { "Revoke \($0)" }
    public var revokeTitle = "Revoke this client's access?"
    public var revokeMessage = "This client will lose access immediately. This cannot be undone from here. It would need a brand new token to reconnect."
    public var cancel = "Cancel"
    // Token sheet
    public var tokenTitle = "Client token created"
    public var tokenMessage = "This is the only time this token will be shown. Copy it now. Whimbrel only stores its hash, so it cannot be recovered later."
    public var tokenName = "Token"
    public var copy = "Copy to Clipboard"
    public var copied = "Copied"
    public var done = "Done"
    public var tokenCloseQuestion = "This token is not shown again. Close without copying it?"
    public var closeWithoutCopying = "Close without copying"
    public var keepOpen = "Keep it open"
    public init() {}
}

public struct WAutomationsLabels: Sendable {
    public var heading = "Routines"
    public var intro = "Scheduled automations that run on their own and deliver a notification when done."
    public var runsOn = "Routines run on"
    public var noRoutines = "No routines yet."
    public var autoApprove = "Auto-approve write actions"
    public var runsDailyAt = "Runs daily at"
    public var timePlaceholder = "HH:MM"
    public var timeInvalid = "Use a time like 07:30."
    public var prepareLeadNote: @Sendable (Int) -> String = { "Prepare a brief \($0) minutes before each meeting" }
    public var minutes: @Sendable (Int) -> String = { "\($0) min" }
    public var model = "Model"
    public var cloudToggle = "Run in the cloud"
    public var readyBy = "Ready by"
    public init() {}
}
