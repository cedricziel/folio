import Foundation
import OpenAPIRuntime

/// The server serializes timestamps with fractional seconds only when they are non-zero,
/// so decoding must accept both forms.
struct LenientDateTranscoder: DateTranscoder {
    func encode(_ date: Date) throws -> String {
        try ISO8601DateTranscoder.iso8601WithFractionalSeconds.encode(date)
    }

    func decode(_ dateString: String) throws -> Date {
        do {
            return try ISO8601DateTranscoder.iso8601WithFractionalSeconds.decode(dateString)
        } catch {
            return try ISO8601DateTranscoder.iso8601.decode(dateString)
        }
    }
}
