import Foundation

/// The notes a meeting's summary (and its stored "Your notes") is built from.
///
/// Two places hold what the user typed while recording: the in-memory Library header editor
/// (`AppState.meetingNotes`, handed to the transcription job) and the Today screen's live note
/// field, which autosaves to `meeting_note_draft.personal_notes`. The summary must see both.
///
/// Rule (`combined`): Library text first, then the draft's lines that Library does not already
/// hold. A draft line is dropped only when its trimmed text matches a not yet matched Library
/// line, one for one, so identical text is never duplicated and a line that differs even by a
/// word is kept. Blank draft lines only separate kept lines. Nothing from either side is lost
/// except an exact repeat. The result is stable: combining the result with the same draft again
/// returns it unchanged, so the job may run again (transcribe again, summarize again, launch
/// recovery, a continued meeting whose stored notes already include its earlier draft) without
/// the draft piling up.
enum LiveNotesMerge {
    static func combined(library: String?, draft: String?) -> String {
        let lib = library ?? ""
        let today = draft ?? ""
        if isBlank(today) { return lib }
        if isBlank(lib) { return today }

        var held: [String: Int] = [:]
        for line in lib.split(separator: "\n", omittingEmptySubsequences: true) {
            let key = line.trimmingCharacters(in: .whitespaces)
            if !key.isEmpty { held[key, default: 0] += 1 }
        }
        var kept: [String] = []
        for line in today.split(separator: "\n", omittingEmptySubsequences: false) {
            let text = String(line)
            let key = text.trimmingCharacters(in: .whitespaces)
            if key.isEmpty {
                kept.append(text)
            } else if let n = held[key], n > 0 {
                held[key] = n - 1
            } else {
                kept.append(text)
            }
        }
        // A blank line only counts between kept lines.
        while let first = kept.first, isBlank(first) { kept.removeFirst() }
        while let last = kept.last, isBlank(last) { kept.removeLast() }
        // Collapse blank runs that dropped lines left behind.
        var tidy: [String] = []
        for line in kept where !(isBlank(line) && (tidy.last.map(isBlank) ?? false)) { tidy.append(line) }
        if tidy.isEmpty { return lib }
        return lib + "\n" + tidy.joined(separator: "\n")
    }

    private static func isBlank(_ s: String) -> Bool {
        s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
