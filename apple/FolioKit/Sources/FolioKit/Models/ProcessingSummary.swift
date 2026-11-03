public struct ProcessingSummary: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let status: String
    public let detail: String?
}

public extension Document {
    var processingSummary: [ProcessingSummary] {
        [
            ProcessingSummary(
                id: "ocr", title: "Text extraction",
                status: ocrStatus == .done && content == ""
                    ? "Completed with no text" : ocrStatus.processingLabel(running: "Reading"),
                detail: ocrStatus == .failed ? "Text extraction did not finish. The original file is unchanged." : nil
            ),
            ProcessingSummary(
                id: "embedding", title: "Search indexing",
                status: embedStatus.processingLabel(running: "Indexing"),
                detail: embedStatus == .failed ? "Search indexing did not finish. The original file is unchanged." : nil
            ),
        ]
    }
}

private extension Document.ProcessingStatus {
    func processingLabel(running: String) -> String {
        switch self {
        case .pending: "Waiting"
        case .running: running
        case .done: "Completed"
        case .failed: "Last attempt failed"
        case .skipped: "Skipped"
        }
    }
}
