import FolioAPI
import Foundation
import Testing

@testable import FolioKit

struct WorkflowCadenceDraftTests {
    @Test(
        arguments: [
            ("workflow.scheduled", #"{"type":"once","at":"2032-02-29T09:30:00.123456789Z"}"#),
            ("workflow.scheduled", #"{"type":"once","at":"2016-12-31T23:59:60Z"}"#),
            ("document.date_offset", #"{"type":"document_date_offset","days":-3,"time_utc":"09:30:00.123456789"}"#),
            (
                "workflow.scheduled",
                #"{"type":"recurring","first_at":"2032-02-29T09:30:00.123456789Z","interval_seconds":4294967295}"#
            ),
        ])
    func savedCadenceRoundTrips(trigger: String, body: String) throws {
        let saved = try JSONDecoder().decode(Components.Schemas.WorkflowCadence.self, from: Data(body.utf8))
        let draft = WorkflowCadenceDraft(saved)
        #expect(try draft.payload(for: trigger) == saved)
        #expect(try draft.payload(for: "document.uploaded") == nil)
    }

    @Test(arguments: [
        "2031-02-29T09:30:00Z", "2032-02-29T24:00:00Z", "2032-02-29T09:30:00+01:00",
        "2032-02-29T09:30:00.1234567890Z", "0000-01-01T09:30:00Z",
    ])
    func invalidInstantCannotBeSaved(_ instant: String) {
        var draft = WorkflowCadenceDraft()
        draft.start = instant
        #expect(!draft.isValid(for: "workflow.scheduled"))
    }

    @Test(arguments: ["0", "-1", "4294967296", "1.5"])
    func invalidRepeatIntervalCannotBeSaved(_ interval: String) {
        var draft = WorkflowCadenceDraft()
        draft.start = "2032-02-29T09:30:00Z"
        draft.frequency = .recurring
        draft.interval = interval
        #expect(!draft.isValid(for: "workflow.scheduled"))
    }

    @Test func dateOffsetAcceptsMinutesAndValidatesCalendarRange() throws {
        var draft = WorkflowCadenceDraft()
        draft.days = "-2"
        draft.time = "09:30"
        let trigger = "document.date_offset"
        let payload = try #require(try draft.payload(for: trigger))
        guard case .case3(let offset) = payload else {
            Issue.record("Expected a document date offset")
            return
        }
        #expect(offset.days == -2)
        #expect(offset.timeUtc == "09:30:00")
        draft.days = "-3652059"
        #expect(!draft.isValid(for: trigger))
        draft.days = "0"
        draft.time = "23:59:60"
        #expect(!draft.isValid(for: trigger))
    }

    @Test func aLeapSecondCannotStartARecurringClock() {
        var draft = WorkflowCadenceDraft()
        draft.start = "2016-12-31T23:59:60Z"
        draft.frequency = .recurring
        draft.interval = "3600"
        #expect(
            draft.validationMessage(for: "workflow.scheduled") == "Recurring clocks cannot start during a leap second")
        draft.frequency = .once
        #expect(draft.isValid(for: "workflow.scheduled"))
    }
}
