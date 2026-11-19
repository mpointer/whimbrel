import Foundation

/// Every user-facing string of the encryption screens (X4 Task 10), in one place so a test
/// can hold them to the design's copy rules (section 7): encryption is scoped to "the memory
/// database", the words are said to be the raw key, nothing is called safe that is not, no
/// jargon, no em or en dashes. English only until Mike asks for translations.
enum EncryptionCopy {
    static let recoveryFileName = "Whimbrel recovery key.txt"

    // MARK: Setup

    static let setupTitle = String(localized: "Create your recovery key")
    static let setupIntro = String(localized: "Whimbrel will encrypt the memory database on this Mac with a key kept in your Mac Keychain. If that key is ever lost, your recovery key is the only way back into the memory database. Make it now, before anything is encrypted.")
    static let stepPassphrase = String(localized: "1. Choose a passphrase")
    static let passphraseHelp = String(localized: "At least 12 characters. A short sentence you will remember works well. The passphrase locks the recovery file and nothing else.")
    static let passphraseField = String(localized: "Passphrase")
    static let confirmField = String(localized: "Type it again")
    static func passphraseCount(_ count: Int, _ minimum: Int) -> String {
        String(localized: "\(count) of at least \(minimum) characters")
    }
    static func passphraseTooShort(_ minimum: Int) -> String {
        String(localized: "Use at least \(minimum) characters.")
    }
    static let passphrasesDiffer = String(localized: "The two passphrases do not match.")
    static let passphraseLockedHint = String(localized: "To use a different passphrase, start over.")
    static let startOver = String(localized: "Start over…")
    static let startOverTitle = String(localized: "Start over with a new recovery key?")
    static let startOverBody = String(localized: "Any recovery file or printout you already made will not work with the new one.")
    static let startOverReuseTitle = String(localized: "Change the passphrase for your recovery file?")
    static let startOverReuseBody = String(localized: "Your 24 words stay the same. A recovery file you already saved still opens with its old passphrase.")
    static let startOverConfirm = String(localized: "Start over")
    static let keepThisKey = String(localized: "Keep this one")
    static let createKey = String(localized: "Create recovery key")
    static let creatingKey = String(localized: "Creating the recovery key…")
    static let createFailed = String(localized: "The recovery key could not be made. Try again.")

    static let stepFile = String(localized: "2. Save the recovery file")
    static let fileHelp = String(localized: "Save Whimbrel recovery key.txt somewhere other than this Mac, such as a USB drive, or attached to an entry in your password manager. It is locked with your passphrase. If you forget the passphrase, the file cannot be opened.")
    static let saveFile = String(localized: "Save recovery file…")
    static func savedTo(_ path: String) -> String { String(localized: "Saved to \(path)") }
    static let saveFailed = String(localized: "The file could not be saved there. Try another folder.")
    static let sameDiskWarning = String(localized: "This file is on the same disk as your memory database, so it may be lost with it. Keep a copy somewhere other than this Mac as well.")

    static let stepWords = String(localized: "3. Write down your 24 recovery words")
    static let wordsAreTheKey = String(localized: "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.")
    static let printWords = String(localized: "Print words and QR code…")
    static let printedNote = String(localized: "Printed.")
    static let printAsFileRefused = String(localized: "Saving the page as a PDF would leave your recovery words as an ordinary file on this Mac. Print it on paper instead, or write the words down.")
    static let wroteDown = String(localized: "I wrote down all 24 words, in order")
    static let qrLabel = String(localized: "QR code of the 24 words")

    static let stepTest = String(localized: "4. Test your passphrase")
    static let testHelp = String(localized: "Type your passphrase once more. Whimbrel checks it against the recovery file it just made, so you know your passphrase opens the recovery file.")
    static let testButton = String(localized: "Test")
    static let testing = String(localized: "Checking…")
    static let testPassed = String(localized: "That passphrase opens your recovery file.")
    static let testFailed = String(localized: "That passphrase does not open the recovery file. Check it and try again.")
    static let iSavedIt = String(localized: "I have kept my recovery file or my 24 words somewhere other than this Mac")
    static let iSavedItLocked = String(localized: "First pass the test above, and save the recovery file, print the words or tick that you wrote them down.")
    static let lostBoth = String(localized: "You only need the recovery key if this Mac loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.")
    static let continueDefault = String(localized: "Continue")

    static let printTitle = String(localized: "Whimbrel recovery words")
    static let printFooter = String(localized: "Keep this page somewhere safe and offline. Anyone who has it can read the memory database of the Mac it was made on.")

    // MARK: Recovery (the key is gone or does not open the database)

    static let recoveryTitle = String(localized: "Whimbrel can't open your memory database")
    static let recoveryKeyMissing = String(localized: "This Mac has no key for the encrypted memory database. That happens after a new user account, a Keychain reset, or when the data folder came from another Mac.")
    static let recoveryKeyRejected = String(localized: "The key in your Mac Keychain does not open this memory database.")
    static let recoveryUnchanged = String(localized: "Your data has not been changed. Use your recovery key to open it.")
    static let methodFile = String(localized: "Recovery file")
    static let methodWords = String(localized: "24 words")
    static let chooseFile = String(localized: "Choose recovery file…")
    static let unlockWithFile = String(localized: "Unlock with file")
    static let wordsHelp = String(localized: "Type or paste your 24 words. Spaces, line breaks and capitals do not matter.")
    static let unlockWithWords = String(localized: "Unlock with words")
    static let unlocking = String(localized: "Checking…")
    static let wrongPassphrase = String(localized: "That passphrase does not open this recovery file, or the file is damaged. Check the passphrase and try again.")
    static let notARecoveryFile = String(localized: "This is not a Whimbrel recovery file. Choose the file named Whimbrel recovery key.txt.")
    static let fileUnreadable = String(localized: "That file could not be read. Choose it again.")
    static let badWords = String(localized: "Those words are not a valid recovery key. Check that all 24 words are there, in order, and spelled correctly.")
    static let keyForAnotherDatabase = String(localized: "That recovery key does not open this memory database. It may belong to another Mac. Try another recovery file or set of words.")
    static let restoreFailed = String(localized: "The recovery key could not be used. Your data has not been changed. Try again.")
    static let restored = String(localized: "Unlocked. Whimbrel is opening your memory database.")
    static let recoveryLostBoth = String(localized: "You only need the recovery key if this Mac loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.")
    static let recoveryNeverDeletes = String(localized: "Whimbrel will not delete or replace this database. It stays on this Mac, unchanged, until you unlock it.")

    // MARK: Key unavailable (the Keychain could not be read)

    static let unavailableTitle = String(localized: "Whimbrel can't read its key right now")
    static let unavailableBody = String(localized: "macOS did not let Whimbrel read the memory database key from your Keychain. Your key and your data are most likely still there. This is not the same as a lost key, and you do not need your recovery key for it.")
    static let unavailableHint = String(localized: "This can happen while your Mac is locked or just after you sign in. Unlock your Mac, then try again.")
    static func unavailableDetail(_ detail: String) -> String { String(localized: "Details: \(detail)") }
    static let tryAgain = String(localized: "Try again")
    static let trying = String(localized: "Trying…")
    static let stillUnavailable = String(localized: "The key still can't be read. Check that your Mac is unlocked, then try again.")
    static let unavailableTouchesNothing = String(localized: "Whimbrel has not opened, changed or deleted anything.")

    // MARK: Settings

    static let settingsTitle = String(localized: "Database encryption")
    static let stateNotEncrypted = String(localized: "Not encrypted")
    static let stateEncrypted = String(localized: "Encrypted")
    static let stateNeedsAttention = String(localized: "Needs attention")
    static let settingsNotEncryptedBody = String(localized: "The memory database is an ordinary file on this Mac. Encrypting it makes a copy of the file unreadable without a key kept in your Mac Keychain.")
    static let settingsEncryptedBody = String(localized: "The memory database on this Mac is encrypted with a key kept in your Mac Keychain. A copy of the database file is unreadable without it. Backups made before you turned this on, such as Time Machine, still hold a readable copy.")
    static let notEncryptedList = String(localized: "Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files.")
    static let notProtectedAgainst = String(localized: "Encryption does not protect against malware running as you, or someone using your Mac while it is unlocked.")
    static let encryptAction = String(localized: "Encrypt the database…")
    static let exportFirstTitle = String(localized: "Export everything first?")
    static let exportFirstBody = String(localized: "Before encrypting, you can export a readable copy of your data. An export is an ordinary folder and is not encrypted, so keep it somewhere safe.")
    static let exportAction = String(localized: "Export everything…")
    static let continueWithoutExport = String(localized: "Continue without exporting")
    static let continueAfterExport = String(localized: "Continue")
    static let cancel = String(localized: "Cancel")
    static let encryptContinue = String(localized: "Encrypt the memory database")
    static let encrypting = String(localized: "Encrypting the memory database…")
    static let encryptDone = String(localized: "The memory database is now encrypted.")
    static let encryptAtNextLaunch = String(localized: "Whimbrel will finish encrypting the memory database the next time it starts.")
    static let restartAction = String(localized: "Restart Whimbrel")
    static let encryptFailed = String(localized: "Encryption could not be completed. Your data is unchanged.")
    static let dismiss = String(localized: "OK")
    static let checkingFirst = String(localized: "Checking that encryption can start…")
    static let plainBackupNotice = String(localized: "Whimbrel keeps an unencrypted copy from before encryption until it has started normally three times, then deletes it. Deleting a file does not guarantee its contents are gone from the disk.")
    static let skipRecording = String(localized: "A meeting is being recorded. Try again when it ends. Your data is unchanged.")
    static let skipLowDisk = String(localized: "There is not enough free disk space to encrypt the memory database. Free up some space and try again.")
    static let skipOpenElsewhere = String(localized: "Another program has the memory database open. Close it and try again.")
    static let skipOther = String(localized: "Encryption did not start. Your data is unchanged. Try again later.")
    static let attentionNeedsKey = String(localized: "This Mac has no key for the encrypted memory database. Use your recovery key to open it.")
    static let attentionKeyRejected = String(localized: "The key in your Keychain does not open the memory database. Use your recovery key to open it.")
    static let attentionKeyUnavailable = String(localized: "Whimbrel could not read its key from your Keychain. Your data is still there.")
    static let attentionMigrationInterrupted = String(localized: "Encrypting the memory database was interrupted. Your data is still there. Restart Whimbrel to finish or undo it.")
    static let attentionDamaged = String(localized: "The memory database file looks damaged. Whimbrel has not changed it.")
    static let attentionStartupFailed = String(localized: "The memory database opened, but Whimbrel could not start with it.")
    static let attentionUnsupported = String(localized: "This copy of Whimbrel cannot open an encrypted database. Install the latest version.")
    static func attentionUnexpected(_ code: Int32) -> String {
        String(localized: "Whimbrel got an unexpected answer when opening the memory database (code \(Int(code))).")
    }

    // MARK: Launch screens (Task 11b-2)

    static let quitAction = String(localized: "Quit Whimbrel")
    static let starting = String(localized: "Starting Whimbrel…")
    static let showWhimbrel = String(localized: "Show Whimbrel")
    static let settingsAfterStart = String(localized: "Settings open once Whimbrel has started.")

    static let migratingTitle = String(localized: "Encrypting the memory database")
    static let migratingBody = String(localized: "This can take a few minutes on a large memory database. Keep Whimbrel open and your Mac awake until it finishes.")
    static let migratingInterruptHint = String(localized: "If it is interrupted, Whimbrel finishes or undoes it the next time it starts.")

    static let keyStorageTitle = String(localized: "Whimbrel can't save its encryption key")
    static let keyStorageBody = String(localized: "Whimbrel keeps the key for the memory database in your Mac Keychain, and saves it there before it creates the database. macOS did not allow that.")
    static let keyStorageHint = String(localized: "Unlock your Mac, or check Keychain Access, then try again. Whimbrel has not created anything.")

    static let interruptedTitle = String(localized: "Encrypting the memory database was interrupted")
    static let interruptedBody = String(localized: "Whimbrel could not finish or undo the encryption it started earlier. Your data is still on this Mac. Try again, and Whimbrel will finish or undo it.")

    static let damagedBody = String(localized: "The memory database file looks damaged, so Whimbrel cannot open it. Whimbrel has not changed it and will not delete or replace it.")
    static let damagedHint = String(localized: "Keep the file as it is, and write to hello@whimbrelapp.com for help.")
    static let noAccessBody = String(localized: "macOS did not let Whimbrel read the memory database file. Whimbrel has not changed it.")
    static let noAccessHint = String(localized: "Check that your user account can open the Whimbrel folder in Application Support, then try again.")
    static let openElsewhereBody = String(localized: "Another program has the memory database file open, so Whimbrel cannot read it. Whimbrel has not changed it.")
    static let openElsewhereHint = String(localized: "Close the other program (a second copy of Whimbrel, a backup tool or antivirus), then try again.")

    static let missingTitle = String(localized: "Whimbrel can't find the memory database")
    static let missingWithBackupBody = String(localized: "The encrypted memory database is missing, but the unencrypted copy from before encryption is still on this Mac. Whimbrel can put that copy back and open it.")
    static let missingWithBackupHint = String(localized: "The copy holds your data from before encryption started. Anything added after that was in the encrypted file. If you can find the encrypted file, for example in a backup, put it back instead and try again.")
    static let restoreCopyAction = String(localized: "Put back the unencrypted copy…")
    static let restoreConfirmTitle = String(localized: "Put back the unencrypted copy?")
    static let restoreConfirmBody = String(localized: "Whimbrel will open the copy from before encryption. It is not encrypted, and anything added after encryption started is not in it.")
    static let restoreConfirm = String(localized: "Put it back")
    static let restorePartlyDone = String(localized: "Some leftover files were moved aside, but the copy was not put back yet. Try again.")
    static let restoreCopyFailed = String(localized: "The copy could not be put back, and some files may have moved. Try again, or quit and write to hello@whimbrelapp.com.")
    static let restoreRefused = String(localized: "The copy could not be put back. Nothing was changed.")
    static let missingEncryptedCopyOnlyBody = String(localized: "The memory database is missing. Only an unfinished encrypted copy is left, and Whimbrel will not change it.")
    static let missingNoCopyResetBody = String(localized: "Whimbrel found no memory database and no copy of it. Reset sets aside Whimbrel's record of the earlier encryption; nothing is deleted.")
    static let missingNoCopyHint = String(localized: "If you have a backup of the memory database, quit Whimbrel, put the backup back, and open Whimbrel again. If not, use Reset. For help, write to hello@whimbrelapp.com.")
    static let resetStatusEllipsis = String(localized: "Reset encryption status…")
    static let resetConfirmTitle = String(localized: "Reset encryption status and start empty?")
    static let resetConfirmBody = String(localized: "Whimbrel will start with an empty memory database. Nothing is deleted: only Whimbrel's record of the earlier encryption is set aside. If you can find the memory database, for example in a backup, quit Whimbrel, put it back, and open Whimbrel again.")
    static let resetConfirm = String(localized: "Reset and start empty")
    static let continueWithoutStore = String(localized: "Continue without the memory database")
    static let needsAttention = String(localized: "Whimbrel needs your attention")
    static let quitSetupTitle = String(localized: "Your recovery file or printout will not work if you quit now. Quit anyway?")
    static let quitSetupBody = String(localized: "The next time Whimbrel starts, it makes a new recovery key with new words.")
    static let keepSettingUp = String(localized: "Keep setting up")
    static let supportNeeded = String(localized: "This needs help from support. Quit Whimbrel, leave the Whimbrel folder in Application Support as it is, and write to hello@whimbrelapp.com.")

    static let errorTitle = String(localized: "Whimbrel couldn't start")
    static let errorStoppedSafely = String(localized: "Whimbrel stopped before opening the memory database, to keep it safe.")
    static let errorTryAgainHint = String(localized: "Try again. If this keeps happening, quit Whimbrel and write to hello@whimbrelapp.com.")
    static let errorFilesUnreadable = String(localized: "Whimbrel could not look at its data folder.")
    static let errorOpenFailed = String(localized: "The memory database could not be opened. Whimbrel has not changed it.")
    static let errorUnexpectedKey = String(localized: "Whimbrel found an encryption key it did not expect, so it did not make a new one. Nothing was created.")
    static let errorKeyGone = String(localized: "The encryption key could not be found while Whimbrel was starting. Nothing was opened or changed.")
    static let errorFileGoneForRecoveryKey = String(localized: "Whimbrel has your recovery key, but the memory database file is not there now. Put the file back, then try again. Whimbrel keeps your recovery key until you quit.")

    // MARK: Settings notes after the launch (Task 11b-2)

    static let noteKeyStoreUnavailable = String(localized: "Whimbrel could not read your Keychain when it started, so encryption cannot start yet. The memory database opened as usual.")
    static let noteRetryScheduled = String(localized: "Whimbrel will try again the next time it starts.")
    static let noteRetryFailed = String(localized: "Whimbrel could not clear the earlier attempts. Try again.")
    static let noteSkippedRecording = String(localized: "Encryption did not start because a meeting was being recorded. Whimbrel will try again the next time it starts.")
    static let noteSkippedLowDisk = String(localized: "Encryption did not start because there was not enough free disk space. Free up some space, and Whimbrel will try again the next time it starts.")
    static let noteSkippedOpenElsewhere = String(localized: "Encryption did not start because another program had the memory database open. Whimbrel will try again the next time it starts.")
    static let noteSkippedOther = String(localized: "Encryption did not start at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.")
    static let noteStateUnreadable = String(localized: "Encryption did not start because Whimbrel cannot read its record of an earlier encryption attempt. Your data is unchanged.")
    static let resetStatusAction = String(localized: "Reset encryption status")
    static let resetStatusFailed = String(localized: "The encryption status could not be reset. Try again.")
    static let noteMigrationFailed = String(localized: "Encryption could not be completed at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.")
    static let noteEncryptionAvailable = String(localized: "This Mac already has an encryption key from an earlier attempt. Encrypting the database uses it, so recovery words you kept for it still work.")
    static let notePlainBackupKept = String(localized: "An unencrypted copy of the memory database from before encryption is still on this Mac.")
    static let deletePlainBackupAction = String(localized: "Delete the unencrypted copy")
    static let deleteNotGuaranteed = String(localized: "Deleting a file does not guarantee its contents are gone from the disk.")
    static let deletePlainBackupFailed = String(localized: "The unencrypted copy could not be deleted. Try again.")
    static let noteRestoredCopy = String(localized: "Whimbrel put back the unencrypted copy of the memory database. It is not encrypted.")
    static let noteRestoredMovedAside = String(localized: "Leftover files that were in the way were moved aside next to it.")
    static let noteRestoredResetHint = String(localized: "If encryption does not start later, use Reset encryption status.")
    static let noteRecoveredKeyNotStored = String(localized: "Your recovery key opened the memory database, but Whimbrel could not save it in your Keychain. Keep your recovery key at hand: Whimbrel will ask for it again the next time it starts.")
    static let noteRecoveredKeyNoSlot = String(localized: "Your recovery key opened the memory database, but your Keychain already holds as many old Whimbrel keys as Whimbrel keeps, so it was not saved. Whimbrel will ask for it again the next time it starts.")
    static let noteRecoveredKeyUnverified = String(localized: "Your recovery key opened the memory database, but Whimbrel could not confirm it was saved in your Keychain. If Whimbrel asks for it the next time it starts, use it again.")
    static let notePendingKeyNotPromoted = String(localized: "Whimbrel could not finish saving its encryption key in your Keychain. It will try again the next time it starts. The memory database opened normally.")
    static let notePendingKeyKept = String(localized: "Whimbrel kept a spare encryption key it found in your Keychain. The memory database opened normally.")

    // MARK: Settings: Faster search and the speed check (Task 11b-2)

    static let precheckKeychainUnavailable = String(localized: "Whimbrel can't read your Keychain right now, so encryption cannot start. Unlock your Mac, then try again.")
    static let fastSearchToggle = String(localized: "Faster search (uses more memory)")
    static let fastSearchHelp = String(localized: "Gives search up to 1 GB of extra memory. Takes effect after you restart Whimbrel, and only applies once the memory database is encrypted.")
    static let speedCheckAction = String(localized: "Check encryption speed")
    static let speedChecking = String(localized: "Checking speed…")
    static let speedCheckPrivacy = String(localized: "The report stays on this Mac. It holds timings and the kind of Mac only, no content, and you decide whether to send it.")
    static let speedCheckRecording = String(localized: "A meeting is being recorded. Check the speed when it ends.")
    static let twoKeysFound = String(localized: "This Mac holds two different encryption keys from earlier attempts, so encryption cannot start. Your data is unchanged. Write to hello@whimbrelapp.com for help.")
    static let restartFailed = String(localized: "Whimbrel could not restart by itself. Quit Whimbrel and open it again.")
    static let restartStuck = String(localized: "Whimbrel did not close in time, so it was not opened again. Quit Whimbrel and open it again.")
    static let noStoreNotice = String(localized: "The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")
    static let recordingBlockedNoStore = String(localized: "Whimbrel can't record meetings because the memory database could not be opened.")
    static let recordingNotSaved = String(localized: "Whimbrel didn't start recording because the meeting could not be saved to the memory database. Try again in a moment.")
    static let recordingOffCaption = String(localized: "Recording is off because the memory database could not be opened.")
    static let openSettingsAction = String(localized: "Open Settings")
    static let noteStoreUnavailable = String(localized: "The memory database could not be opened.")
    static let noteStoreUnavailableHint = String(localized: "Whimbrel is running without the memory database, so nothing new is saved. Restart Whimbrel to try again.")
    static let speedCheckFailed = String(localized: "The speed check could not finish. Try again.")
    static let speedNotEnoughData = String(localized: "Not enough data to time searches yet.")
    static let speedEncrypted = String(localized: "Memory database: encrypted")
    static let speedNotEncrypted = String(localized: "Memory database: not encrypted")
    static func speedOpen(_ ms: String) -> String { String(localized: "Opening: \(ms) ms") }
    static func speedKeyword(_ typical: String, _ slow: String) -> String {
        String(localized: "Word search: \(typical) ms typical, \(slow) ms at the slow end")
    }
    static func speedSemantic(_ typical: String, _ slow: String) -> String {
        String(localized: "Search by meaning: \(typical) ms typical, \(slow) ms at the slow end")
    }
    static let speedSemanticUnavailable = String(localized: "Search by meaning: not timed, the search model is not available")
    static func speedComputer(_ arch: String, _ ramGB: String, _ os: String) -> String {
        String(localized: "This Mac: \(arch), \(ramGB) GB of memory, \(os)")
    }
    static let speedNotMeasured = String(localized: "not measured")
    static let saveReportAction = String(localized: "Save report…")
    static let speedReportFileName = "Whimbrel encryption speed.txt"

    // MARK: Task 13c (Mike's decisions 2026-11-16)

    /// On the first onboarding screen after Delete everything only.
    static let deletedLibraryLine = String(localized: "Your previous data was deleted. This is a new, empty library.")
    /// Once, after a launch that found neither the database nor a copy where a store had been.
    static let libraryMissingNotice = String(localized: "Your library couldn't be found, so Whimbrel started a new, empty one. If you have a backup of your library, the help page \"Restoring a library backup\" explains how to put it back.")
    /// The notice's link (the app's convention for help: a link to a whimbrelapp.com page).
    static let libraryBackupHelpLink = String(localized: "Restoring a library backup")
    static let libraryBackupHelpURL = URL(string: "https://whimbrelapp.com/help/restore-library-backup/")!
    /// Settings > Your data, while set-aside `.foreign-*` files exist.
    static func foreignFilesSummary(count: Int, size: String) -> String {
        count == 1 ? String(localized: "1 file kept aside, \(size)") : String(localized: "\(count) files kept aside, \(size)")
    }
    static let foreignFilesHelp = String(localized: "Files Whimbrel set aside beside the memory database while undoing or repairing encryption. Whimbrel does not use them.")
    static let foreignFilesDeleteAction = String(localized: "Delete these files")
    static let foreignFilesConfirmTitle = String(localized: "Delete these files?")
    static func foreignFilesConfirmBody(count: Int, size: String) -> String {
        count == 1
            ? String(localized: "This deletes 1 file kept aside (\(size)). It cannot be recovered.")
            : String(localized: "This deletes \(count) files kept aside (\(size)). They cannot be recovered.")
    }
    static let foreignFilesConfirm = String(localized: "Delete")
    static let foreignFilesNotDeleted = String(localized: "These files could not be deleted:")
    static let foreignFilesUnreadable = String(localized: "Whimbrel could not read the data folder, so nothing was deleted. Try again.")

    /// Delete everything could not remove the `store_existed` marker (fix D): said at launch,
    /// with its path, before the next launch would wrongly say the library could not be found.
    static let storeMarkerNotRemoved = String(localized: "Delete everything could not remove this Whimbrel file. Quit Whimbrel and remove it by hand, or Whimbrel may say your library could not be found:")

    private static var task13c: [String] {
        [deletedLibraryLine, libraryMissingNotice, libraryBackupHelpLink, storeMarkerNotRemoved, foreignFilesSummary(count: 2, size: "1 MB"), foreignFilesSummary(count: 1, size: "4 KB"),
         foreignFilesHelp, foreignFilesDeleteAction, foreignFilesConfirmTitle, foreignFilesConfirmBody(count: 3, size: "1.2 GB"),
         foreignFilesConfirmBody(count: 1, size: "4 KB"), foreignFilesConfirm, foreignFilesNotDeleted, foreignFilesUnreadable]
    }

    /// Every string above (interpolated ones with sample values), for the copy-rule test.
    static var all: [String] {
        [
            quitAction, starting, showWhimbrel, settingsAfterStart, migratingTitle, migratingBody, migratingInterruptHint,
            keyStorageTitle, keyStorageBody, keyStorageHint, interruptedTitle, interruptedBody,
            damagedBody, damagedHint, noAccessBody, noAccessHint, openElsewhereBody, openElsewhereHint, missingTitle, missingWithBackupBody, missingWithBackupHint,
            restoreCopyAction, restoreConfirmTitle, restoreConfirmBody, restoreConfirm, restorePartlyDone, restoreCopyFailed,
            restoreRefused, missingEncryptedCopyOnlyBody, missingNoCopyResetBody, missingNoCopyHint, resetStatusEllipsis, resetConfirmTitle, resetConfirmBody,
            resetConfirm, continueWithoutStore, needsAttention, quitSetupTitle, quitSetupBody, keepSettingUp, supportNeeded, errorTitle, errorStoppedSafely,
            errorTryAgainHint, errorFilesUnreadable, errorOpenFailed, errorUnexpectedKey, errorKeyGone, errorFileGoneForRecoveryKey,
            noteKeyStoreUnavailable, noteRetryScheduled, noteRetryFailed, noteSkippedRecording, noteSkippedLowDisk,
            noteSkippedOpenElsewhere, noteSkippedOther, noteStateUnreadable, resetStatusAction, resetStatusFailed,
            noteMigrationFailed, noteEncryptionAvailable, notePlainBackupKept, deletePlainBackupAction, deleteNotGuaranteed,
            deletePlainBackupFailed, noteRestoredCopy, noteRestoredMovedAside, noteRestoredResetHint, noteRecoveredKeyNotStored,
            noteRecoveredKeyNoSlot, noteRecoveredKeyUnverified, notePendingKeyNotPromoted, notePendingKeyKept,
            precheckKeychainUnavailable, fastSearchToggle, fastSearchHelp, speedCheckAction, speedChecking, speedCheckPrivacy,
            speedCheckRecording, twoKeysFound, noStoreNotice, recordingBlockedNoStore, recordingNotSaved, recordingOffCaption, openSettingsAction, restartFailed, restartStuck, noteStoreUnavailable, noteStoreUnavailableHint,
            speedCheckFailed, speedNotEnoughData, speedEncrypted, speedNotEncrypted, speedOpen("12"), speedKeyword("3.1", "9.4"),
            speedSemantic("40", "80"), speedSemanticUnavailable, speedComputer("arm64", "32", "macOS 26.1"), speedNotMeasured,
            saveReportAction,
        ] + task10 + task13c
    }

    private static var task10: [String] {
        [
            setupTitle, setupIntro, stepPassphrase, passphraseHelp, passphraseField, confirmField,
            passphraseCount(5, 12), passphraseTooShort(12), passphrasesDiffer, passphraseLockedHint, startOver, startOverTitle, startOverBody, startOverConfirm, keepThisKey, startOverReuseTitle, startOverReuseBody, createKey, creatingKey, createFailed,
            stepFile, fileHelp, saveFile, savedTo("/Volumes/USB"), saveFailed, sameDiskWarning,
            stepWords, wordsAreTheKey, printWords, printedNote, printAsFileRefused, wroteDown, qrLabel,
            stepTest, testHelp, testButton, testing, testPassed, testFailed, iSavedIt, iSavedItLocked, lostBoth, continueDefault,
            printTitle, printFooter,
            recoveryTitle, recoveryKeyMissing, recoveryKeyRejected, recoveryUnchanged, methodFile, methodWords, chooseFile,
            unlockWithFile, wordsHelp, unlockWithWords, unlocking, wrongPassphrase, notARecoveryFile, fileUnreadable, badWords,
            keyForAnotherDatabase, restoreFailed, restored, recoveryLostBoth, recoveryNeverDeletes,
            unavailableTitle, unavailableBody, unavailableHint, unavailableDetail("status -25308"), tryAgain, trying,
            stillUnavailable, unavailableTouchesNothing,
            settingsTitle, stateNotEncrypted, stateEncrypted, stateNeedsAttention, settingsNotEncryptedBody,
            settingsEncryptedBody, notEncryptedList, notProtectedAgainst, encryptAction, exportFirstTitle, exportFirstBody,
            exportAction, continueWithoutExport, continueAfterExport, cancel, encryptContinue, encrypting, encryptDone,
            encryptAtNextLaunch, restartAction, encryptFailed, dismiss, checkingFirst, plainBackupNotice, skipRecording, skipLowDisk, skipOpenElsewhere, skipOther,
            attentionNeedsKey, attentionKeyRejected, attentionKeyUnavailable, attentionMigrationInterrupted,
            attentionDamaged, attentionStartupFailed, attentionUnsupported, attentionUnexpected(42),
        ]
    }
}
