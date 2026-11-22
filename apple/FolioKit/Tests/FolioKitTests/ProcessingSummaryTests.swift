import Foundation
import Testing

@testable import FolioKit

struct ProcessingSummaryTests {
    @Test func emptyOcrCompletionIsNotFailureOrSkippedWork() {
        let document = Document(title: "Scan", content: "", ocrStatus: .done, embedStatus: .skipped)
        #expect(document.processingSummary[0].status == "Completed with no text")
        #expect(document.processingSummary[1].status == "Skipped")
    }

    @Test func indexingFailureDoesNotDescribeTheOriginalAsUnreadable() {
        let document = Document(title: "Scan", content: "Text", ocrStatus: .done, embedStatus: .failed)
        #expect(document.processingSummary[0].status == "Completed")
        #expect(document.processingSummary[1].title == "Search indexing")
        #expect(document.processingSummary[1].status == "Last attempt failed")
        #expect(
            document.processingSummary[1].detail == "Search indexing did not finish. The original file is unchanged.")
    }

    @Test func activeStagesRemainDistinct() {
        let document = Document(title: "Scan", ocrStatus: .running, embedStatus: .pending)
        #expect(document.processingSummary[0].status == "Reading")
        #expect(document.processingSummary[1].status == "Waiting")
    }

    @Test func persistedRetryOverridesLegacyFailureAndRetainsItsSchedule() {
        let retryAt = Date(timeIntervalSince1970: 1_800_000_000)
        let document = Document(
            title: "Scan", ocrStatus: .failed, embedStatus: .done,
            processingStages: [
                ProcessingStageSummary(
                    runID: UUID(), stage: .ocr, attempt: 2, state: .waitingToRetry,
                    reason: .providerFailed, retryAt: retryAt
                )
            ]
        )
        #expect(document.processingSummary[0].status == "Waiting to retry")
        #expect(document.processingSummary[0].attempt == 2)
        #expect(document.processingSummary[0].retryAt == retryAt)
        #expect(document.processingSummary[0].detail == "The processing service could not complete this attempt.")
        #expect(document.processingSummary[1].status == "Completed")
    }

    @Test func currentCompletionUsesItsTextResultInsteadOfPreviousContent() {
        let document = Document(
            title: "Scan", content: "Old extracted text", ocrStatus: .done,
            processingStages: [
                ProcessingStageSummary(runID: UUID(), stage: .ocr, attempt: 1, state: .completed, hasText: false)
            ]
        )
        #expect(document.processingSummary[0].status == "Completed with no text")
    }

    @Test func currentFailureAndThumbnailSkipAreTerminalStates() {
        let document = Document(
            title: "Scan", ocrStatus: .done,
            processingStages: [
                ProcessingStageSummary(
                    runID: UUID(), stage: .embedding, attempt: 3, state: .failed, reason: .storageFailed),
                ProcessingStageSummary(
                    runID: UUID(), stage: .thumbnail, attempt: 1, state: .skipped, reason: .unsupportedFormat),
            ]
        )
        #expect(document.processingSummary[1].status == "Failed")
        #expect(document.processingSummary[1].retryAt == nil)
        #expect(document.processingSummary[2].title == "Page previews")
        #expect(document.processingSummary[2].status == "Skipped")
        #expect(document.processingSummary[2].detail == "Page previews are unavailable for this file type.")
    }

}
