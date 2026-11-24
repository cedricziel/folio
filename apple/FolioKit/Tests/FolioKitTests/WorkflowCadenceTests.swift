import FolioAPI
import Foundation
import Testing

struct WorkflowCadenceTests {
    @Test(arguments: [
        #"{"type":"once","at":"2030-01-03T09:00:00.123456789Z"}"#,
        #"{"type":"once","at":"2016-12-31T23:59:60Z"}"#,
        #"{"type":"recurring","first_at":"2030-01-03T09:00:00.123456789Z","interval_seconds":4294967295}"#,
    ])
    func generatedClientPreservesTheClockInstant(cadence: String) throws {
        let json = """
            {"name":"Scheduled export","trigger":"workflow.scheduled","cadence":\(cadence),
             "actions":[{"type":"webhook","url":"https://example.test/clock"}]}
            """
        let input = try JSONDecoder().decode(Components.Schemas.WorkflowInput.self, from: Data(json.utf8))
        #expect(input.trigger?.rawValue == "workflow.scheduled")
        let encoded = try JSONEncoder().encode(input)
        let result = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        let actual = try #require(result["cadence"] as? NSDictionary)
        let expected = try #require(JSONSerialization.jsonObject(with: Data(cadence.utf8)) as? NSDictionary)
        #expect(actual == expected)
    }
}
