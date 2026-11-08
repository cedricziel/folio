import Foundation

/// A 409 for an upload sent with an `Idempotency-Key`, told apart by the error `code` in the server's answer.
public enum KeyedUploadConflict: Error, Equatable, LocalizedError, Sendable {
    /// An earlier request with the key is still storing the file; try again after `retryAfter`.
    case inProgress(retryAfter: Duration?)
    /// The key was already used for a different file. Retrying with it never succeeds.
    case keyReused

    /// Reads a 409 answer. A 409 without a code, as from servers before the codes, counts as still in progress.
    /// Returns nil for any other status and for a code this client does not know, so the caller reports the
    /// server's own message.
    public init?(response: HTTPURLResponse, body: Data?) {
        guard response.statusCode == 409 else { return nil }
        switch body.flatMap({ try? JSONDecoder().decode(ErrorCode.self, from: $0) })?.code {
        case "idempotency_key_mismatch":
            self = .keyReused
        case nil, "upload_in_progress":
            let seconds = response.value(forHTTPHeaderField: "Retry-After").flatMap { Int($0) }
            self = .inProgress(retryAfter: seconds.map { .seconds(max($0, 0)) })
        default:
            return nil
        }
    }

    public var errorDescription: String? {
        switch self {
        case .inProgress: "Folio is still filing an earlier attempt of this upload. Try again in a moment."
        case .keyReused: "Folio refused this upload because its retry key was used for a different file."
        }
    }

    private struct ErrorCode: Decodable {
        let code: String?
    }
}
