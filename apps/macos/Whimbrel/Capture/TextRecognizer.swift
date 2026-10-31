import CoreGraphics
import Vision

/// Vision OCR on an in-memory image. Shared by window-capture fallback and meeting snapshots.
enum TextRecognizer {
    /// Accurate, language-corrected OCR that detects the text's language (slides and windows
    /// may not be English).
    static func makeRequest() -> VNRecognizeTextRequest {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.automaticallyDetectsLanguage = true
        return request
    }

    static func recognize(_ image: CGImage) -> String? {
        let request = makeRequest()

        let handler = VNImageRequestHandler(cgImage: image, options: [:])
        do {
            try handler.perform([request])
        } catch {
            return nil
        }

        guard let observations = request.results else { return nil }
        let lines = observations.compactMap { $0.topCandidates(1).first?.string }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }
}
