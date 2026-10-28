import ApplicationServices
import AVFoundation
import CoreGraphics
import Foundation
import Security

/// Debug-only: prints the running process's own code signature and raw permission-check
/// results straight to the Xcode console, so a TCC mismatch can be diagnosed without
/// relying on what System Settings' UI shows (which can lag or show a different build).
enum DiagnosticsPrinter {
    static func printSigningAndPermissionState() {
        var teamID = "(unavailable)"
        var signingID = "(unavailable)"
        var isAdHoc = "(unavailable)"

        var code: SecCode?
        if SecCodeCopySelf(SecCSFlags(rawValue: 0), &code) == errSecSuccess, let code {
            var infoRef: CFDictionary?
            let staticCode = code as! SecStaticCode
            if SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &infoRef) == errSecSuccess,
               let info = infoRef as? [String: Any] {
                teamID = info[kSecCodeInfoTeamIdentifier as String] as? String ?? "(none — ad-hoc signature)"
                signingID = info[kSecCodeInfoIdentifier as String] as? String ?? "(unknown)"
                if let flags = info["flags"] as? UInt32 {
                    isAdHoc = (flags & 0x2 != 0) ? "YES (adhoc flag set)" : "no"
                }
            }
        }

        let message = """

        ---- Whimbrel diagnostics ----
        Bundle path:                \(Bundle.main.bundlePath)
        Executable path:             \(Bundle.main.executablePath ?? "?")
        Code signing identifier:     \(signingID)
        Code signing Team ID:        \(teamID)
        Ad-hoc signed:                \(isAdHoc)
        CGPreflightScreenCaptureAccess(): \(CGPreflightScreenCaptureAccess())
        AXIsProcessTrusted():             \(AXIsProcessTrusted())
        Microphone authorizationStatus:   \(AVCaptureDevice.authorizationStatus(for: .audio).rawValue) (3 = authorized)
        ---------------------------------

        """
        print(message)
    }
}
