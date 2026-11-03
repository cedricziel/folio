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
}
