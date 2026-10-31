import AVFoundation

/// Speaks an answer with the system voice. One answer at a time: speaking a second stops the first.
final class ReadAloud: NSObject, AVSpeechSynthesizerDelegate {
    private let synth = AVSpeechSynthesizer()

    override init() {
        super.init()
        synth.delegate = self
    }

    func speak(_ text: String) {
        stop()
        // Dictation leaves the shared session on `.record`, which is silent for speech.
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .spokenAudio)
        try? session.setActive(true)
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = AVSpeechSynthesisVoice(language: Self.voiceLanguage(outputTag: AppLanguage.currentOutputTag()))
        synth.speak(utterance)
    }

    func stop() { synth.stopSpeaking(at: .immediate) }

    /// The voice speaks the language answers are written in; "" (English) is en-US.
    static func voiceLanguage(outputTag: String) -> String { outputTag.isEmpty ? "en-US" : outputTag }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) { release() }
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) { release() }

    private func release() {
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
