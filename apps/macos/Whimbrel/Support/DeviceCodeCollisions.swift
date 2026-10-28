import Foundation

/// Ids of pending devices whose comparison code is shared with another pending
/// device. Nil codes (no derivable code) never collide.
func collidingPendingDeviceIDs(_ devices: [(id: String, status: String, code: String?)]) -> Set<String> {
    let pending = devices.filter { $0.status == "pending" }
    var byCode: [String: [String]] = [:]
    for d in pending { if let c = d.code { byCode[c, default: []].append(d.id) } }
    return Set(byCode.values.filter { $0.count > 1 }.joined())
}
