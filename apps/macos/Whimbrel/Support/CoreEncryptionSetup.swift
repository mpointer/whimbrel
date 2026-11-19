import Foundation

/// Why a recovery-key call failed (core `WHIMBREL_RECOVERY_*`). The screens map these to
/// fixed English (`EncryptionCopy`); core's own text never reaches the user.
enum CoreRecoveryError: Error, Equatable {
    case passphraseTooShort
    /// The passphrase is wrong or the file was changed; core cannot tell which.
    case wrongPassphraseOrDamaged
    /// Not a recovery file core understands.
    case malformed
    /// Wrong word count, an unknown word, or a checksum failure.
    case badWords
    /// The call was malformed (null pointer, key not 32 bytes). Nothing was done.
    case invalidArgument
    /// A code this build does not know.
    case unknown(Int32)

    init(code: Int32) {
        switch code {
        case WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT: self = .passphraseTooShort
        case WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED: self = .wrongPassphraseOrDamaged
        case WHIMBREL_RECOVERY_MALFORMED: self = .malformed
        case WHIMBREL_RECOVERY_BAD_WORDS: self = .badWords
        case WHIMBREL_RECOVERY_INVALID_ARGUMENT: self = .invalidArgument
        default: self = .unknown(code)
        }
    }
}

/// What recovery setup produced: the key to encrypt with and store in the Keychain, the
/// text of `Whimbrel recovery key.txt` (sealed under the passphrase) and the 24 words (the
/// raw key). Like `DbKeyStore`, Swift `Data`/`String` copies are not wiped.
struct CoreSetupMaterial: Equatable {
    let key: Data
    let recoveryFile: String
    let words: [String]
}

/// Core's recovery-key calls (`api::encrypt_setup` over the C ABI, X4 design 3.4). All in
/// memory: nothing here reads or writes a file or touches the Keychain. `begin`, `verify`
/// and `restore(recoveryFile:)` run Argon2id (about a second): never call them on the main
/// thread.
enum CoreEncryptionSetup {
    static var minimumPassphraseLength: Int { Int(whimbrel_encrypt_min_passphrase_chars()) }

    /// Characters of the NFC form: the count core's minimum applies to.
    static func passphraseLength(_ passphrase: String) -> Int {
        passphrase.withCString { Int(whimbrel_encrypt_passphrase_length($0)) }
    }

    /// A fresh random key (or `existingKey`, the launch flow's pending key, kept as is) with
    /// its recovery file and words.
    static func begin(passphrase: String, existingKey: Data? = nil) -> Result<CoreSetupMaterial, CoreRecoveryError> {
        var key = [UInt8](repeating: 0, count: 32)
        defer { wipe(&key) }
        var file: UnsafeMutablePointer<CChar>?
        var words: UnsafeMutablePointer<CChar>?
        let code: Int32 = passphrase.withCString { p in
            CoreMemoryStore.withKeyBytes(existingKey) { existing, existingLen in
                key.withUnsafeMutableBufferPointer { out in
                    whimbrel_encrypt_setup_begin(p, existing, existingLen, out.baseAddress, out.count, &file, &words)
                }
            }
        }
        defer {
            whimbrel_core_free_secret_string(file)
            whimbrel_core_free_secret_string(words)
        }
        guard code == WHIMBREL_RECOVERY_OK, let file, let words else { return .failure(CoreRecoveryError(code: code)) }
        return .success(CoreSetupMaterial(
            key: Data(key),
            recoveryFile: String(cString: file),
            words: String(cString: words).split(separator: " ").map(String.init)
        ))
    }

    /// The type-it-back test: does `passphrase` open this recovery file text (in memory)?
    static func verify(recoveryFile: String, passphrase: String) -> Bool {
        recoveryFile.withCString { f in passphrase.withCString { p in whimbrel_encrypt_setup_verify_passphrase(f, p) } }
    }

    /// The key from a recovery file's text and its passphrase.
    static func restore(recoveryFile: String, passphrase: String) -> Result<Data, CoreRecoveryError> {
        keyCall { out, n in
            recoveryFile.withCString { f in passphrase.withCString { p in whimbrel_encrypt_restore_from_file(f, p, out, n) } }
        }
    }

    /// The key from the 24 words as typed or pasted (core ignores spacing, capitals and numbering).
    static func restore(words: String) -> Result<Data, CoreRecoveryError> {
        keyCall { out, n in words.withCString { whimbrel_encrypt_restore_from_words($0, out, n) } }
    }

    private static func keyCall(_ body: (UnsafeMutablePointer<UInt8>?, Int) -> Int32) -> Result<Data, CoreRecoveryError> {
        var key = [UInt8](repeating: 0, count: 32)
        defer { wipe(&key) }
        let code = key.withUnsafeMutableBufferPointer { body($0.baseAddress, $0.count) }
        guard code == WHIMBREL_RECOVERY_OK else { return .failure(CoreRecoveryError(code: code)) }
        return .success(Data(key))
    }

    private static func wipe(_ bytes: inout [UInt8]) {
        bytes.withUnsafeMutableBytes { raw in
            guard let base = raw.baseAddress else { return }
            _ = memset_s(base, raw.count, 0, raw.count)
        }
    }
}
