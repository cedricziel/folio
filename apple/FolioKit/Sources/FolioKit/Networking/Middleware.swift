import Foundation
import HTTPTypes
import OpenAPIRuntime

struct BearerAuthMiddleware: ClientMiddleware {
    let provider: any BearerTokenProvider

    func intercept(
        _ request: HTTPRequest,
        body: HTTPBody?,
        baseURL: URL,
        operationID _: String,
        next: @Sendable (HTTPRequest, HTTPBody?, URL) async throws -> (HTTPResponse, HTTPBody?)
    ) async throws -> (HTTPResponse, HTTPBody?) {
        var request = request
        if let token = await provider.token() {
            request.headerFields[.authorization] = "Bearer \(token)"
        }
        return try await next(request, body, baseURL)
    }
}

/// Turns non-2xx answers into `FolioError.server` carrying the server's `{"error": "..."}` message,
/// so callers never have to switch over every documented error response.
struct ErrorMappingMiddleware: ClientMiddleware {
    func intercept(
        _ request: HTTPRequest,
        body: HTTPBody?,
        baseURL: URL,
        operationID _: String,
        next: @Sendable (HTTPRequest, HTTPBody?, URL) async throws -> (HTTPResponse, HTTPBody?)
    ) async throws -> (HTTPResponse, HTTPBody?) {
        let (response, responseBody) = try await next(request, body, baseURL)
        guard response.status.kind != .successful else { return (response, responseBody) }
        var data: Data?
        if let responseBody {
            data = try await Data(collecting: responseBody, upTo: 64 * 1024)
        }
        throw FolioError.server(status: response.status.code, message: data.flatMap(Self.message))
    }

    static func message(in data: Data) -> String? {
        (try? JSONDecoder().decode([String: String?].self, from: data))?["error"] ?? nil
    }
}
