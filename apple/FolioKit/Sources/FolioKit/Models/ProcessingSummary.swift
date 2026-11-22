import Foundation

public struct ProcessingStageSummary: Hashable, Sendable {
    public enum Stage: String, Sendable {
        case ocr, embedding, thumbnail
    }

    public enum State: String, Sendable {
        case pending, running, waitingToRetry = "waiting_to_retry", completed, failed, skipped
    }

    public enum Reason: String, Sendable {
        case providerDisabled = "provider_disabled", noText = "no_text", unsupportedFormat = "unsupported_format"
        case providerFailed = "provider_failed", invalidResponse = "invalid_response"
        case storageFailed = "storage_failed", leaseExpired = "lease_expired"
    }

    public let runID: UUID
    public let revisionID: UUID?
    public let stage: Stage
    public let attempt: Int
    public let state: State
    public let reason: Reason?
    public let retryAt: Date?
    public let hasText: Bool?

    public init(
        runID: UUID, revisionID: UUID? = nil, stage: Stage, attempt: Int, state: State,
        reason: Reason? = nil, retryAt: Date? = nil, hasText: Bool? = nil
    ) {
        self.runID = runID
        self.revisionID = revisionID
        self.stage = stage
        self.attempt = attempt
        self.state = state
        self.reason = reason
        self.retryAt = retryAt
        self.hasText = hasText
    }
}

public struct ProcessingSummary: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let status: String
    public let detail: String?
    public var attempt: Int? = nil
    public var retryAt: Date? = nil
}

public extension Document {
    var processingSummary: [ProcessingSummary] {
        var summaries = [
            processingStages.first { $0.stage == .ocr }?.presentation
                ?? ProcessingSummary(
                    id: "ocr", title: "Text extraction",
                    status: ocrStatus == .done && content == ""
                        ? "Completed with no text" : ocrStatus.processingLabel(running: "Reading"),
                    detail: ocrStatus == .failed
                        ? "Text extraction did not finish. The original file is unchanged." : nil
                ),
            processingStages.first { $0.stage == .embedding }?.presentation
                ?? ProcessingSummary(
                    id: "embedding", title: "Search indexing",
                    status: embedStatus.processingLabel(running: "Indexing"),
                    detail: embedStatus == .failed
                        ? "Search indexing did not finish. The original file is unchanged." : nil
                ),
        ]
        if let thumbnail = processingStages.first(where: { $0.stage == .thumbnail }) {
            summaries.append(thumbnail.presentation)
        }
        return summaries
    }
}

private extension ProcessingStageSummary {
    var presentation: ProcessingSummary {
        let title: String
        let running: String
        switch stage {
        case .ocr: (title, running) = ("Text extraction", "Reading")
        case .embedding: (title, running) = ("Search indexing", "Indexing")
        case .thumbnail: (title, running) = ("Page previews", "Generating")
        }
        let status: String
        switch state {
        case .pending: status = "Waiting"
        case .running: status = running
        case .waitingToRetry: status = "Waiting to retry"
        case .completed: status = stage == .ocr && hasText == false ? "Completed with no text" : "Completed"
        case .failed: status = "Failed"
        case .skipped: status = "Skipped"
        }
        let detail: String?
        switch reason {
        case .providerDisabled: detail = "Disabled in settings."
        case .noText: detail = "There is no extracted text to index."
        case .unsupportedFormat: detail = "Page previews are unavailable for this file type."
        case .providerFailed: detail = "The processing service could not complete this attempt."
        case .invalidResponse: detail = "The processing service returned an unusable result."
        case .storageFailed: detail = "A file could not be read or saved."
        case .leaseExpired: detail = "The processing attempt expired before finishing."
        case nil: detail = state == .failed ? "Processing did not finish. The original file is unchanged." : nil
        }
        return ProcessingSummary(
            id: stage.rawValue, title: title, status: status, detail: detail,
            attempt: attempt > 0 ? attempt : nil, retryAt: retryAt
        )
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
