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
        let token = await provider.token()
        if let token {
            request.headerFields[.authorization] = "Bearer \(token)"
        }
        let first = try await next(request, body, baseURL)
        // A rejected token is refreshed once and the request replayed with the new one.
        guard first.0.status == .unauthorized, let token,
            let fresh = await provider.refresh(rejecting: token)
        else { return first }
        request.headerFields[.authorization] = "Bearer \(fresh)"
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
        if response.status == .unauthorized { throw FolioError.signInRequired }
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
