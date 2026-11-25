/// Words that open a question in the languages the app ships, matched against what a person typed to choose Find's opening
/// order (`WFindOrder.default(for:)`). They are never shown, so they live outside WStrings; this file is exempt from the
/// user-facing literal scan for that reason.
enum WFindQuestionWords {
    static let all: Set<String> = ["who", "what", "where", "when", "why", "how", "which", "whose", "qué", "dónde", "donde", "cuándo", "cuando", "quién", "quien", "cómo", "cuál", "où", "quand", "qui", "comment", "pourquoi", "quel", "quelle", "quels", "quelles", "wo", "wann", "wer", "wie", "warum", "was", "welche", "welcher", "welches", "onde", "quando", "quem", "qual", "quais"]
}
