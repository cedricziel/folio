import FolioAPI
import Foundation

struct WorkflowCadenceDraft {
    enum Trigger: String {
        case clock = "workflow.scheduled"
    }

    enum Frequency: String, CaseIterable, Identifiable {
        case once = "Once"
        case recurring = "Recurring"

        var id: Self { self }
    }

    var frequency: Frequency = .once
    var start = ""
    var interval = ""

    init(_ cadence: Components.Schemas.WorkflowCadence? = nil) {
        switch cadence {
        case .case1(let once):
            start = once.at
        case .case2(let recurring):
            frequency = .recurring
            start = recurring.firstAt
            interval = String(recurring.intervalSeconds)
        case nil:
            break
        }
    }

    func isValid(for triggerID: String) -> Bool {
        validationMessage(for: triggerID) == nil
    }

    func validationMessage(for triggerID: String) -> String? {
        do {
            _ = try payload(for: triggerID)
            return nil
        } catch FolioError.server(_, let message) {
            return message
        } catch {
            return "Check the schedule."
        }
    }

    func payload(for triggerID: String) throws -> Components.Schemas.WorkflowCadence? {
        switch Trigger(rawValue: triggerID) {
        case .clock:
            let instant = start.trimmingCharacters(in: .whitespacesAndNewlines)
            let utcTime = String(instant.dropFirst(11).dropLast())
            guard instant.count >= 20, DocumentDate(rawValue: String(instant.prefix(10))) != nil,
                instant.dropFirst(10).first == "T", instant.hasSuffix("Z"),
                Self.validTime(utcTime, allowsLeapSecond: true)
            else {
                throw FolioError.server(status: 400, message: "Enter a UTC start in YYYY-MM-DDTHH:mm:ssZ format.")
            }
            if frequency == .once { return .case1(.init(at: instant, _type: .once)) }
            guard Self.validTime(utcTime) else {
                throw FolioError.server(status: 400, message: "Recurring clocks cannot start during a leap second")
            }
            guard let seconds = Int64(interval.trimmingCharacters(in: .whitespacesAndNewlines)),
                (1...Int64(UInt32.max)).contains(seconds)
            else {
                throw FolioError.server(status: 400, message: "Enter a repeat interval from 1 to 4294967295 seconds.")
            }
            return .case2(.init(firstAt: instant, intervalSeconds: seconds, _type: .recurring))
        case nil:
            return nil
        }
    }

    private static func validTime(_ time: String, allowsLeapSecond: Bool = false) -> Bool {
        guard let match = time.wholeMatch(of: /(?:[01][0-9]|2[0-3]):[0-5][0-9]:([0-5][0-9]|60)(?:\.[0-9]{1,9})?/)
        else { return false }
        return allowsLeapSecond || match.1 != "60"
    }
}
