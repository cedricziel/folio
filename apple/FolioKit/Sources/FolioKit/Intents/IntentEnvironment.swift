import Foundation
import os

/// Where intents get their server. They run in the app's process, or in a system process that
/// loads the app's code, so the address and sign-in come from the App Group and shared keychain
/// rather than from anything the app holds in memory.
public enum IntentEnvironment {
    public typealias ServiceFactory = @Sendable () throws -> (any FolioService)?

    private static let factory = OSAllocatedUnfairLock<ServiceFactory>(initialState: signedInService)

    /// Swaps the server connection, for tests.
    public static func use(_ make: @escaping ServiceFactory) {
        factory.withLock { $0 = make }
    }

    private static let spotlightIndex = OSAllocatedUnfairLock<SpotlightIndex>(initialState: .shared)

    /// Where intents report documents that are gone, so Spotlight stops offering them.
    public static var spotlight: SpotlightIndex { spotlightIndex.withLock { $0 } }

    /// Swaps the Spotlight index, for tests.
    public static func use(spotlight: SpotlightIndex) {
        spotlightIndex.withLock { $0 = spotlight }
    }

    /// The server intents talk to.
    public static func serverURL() -> URL? {
        SharedConfiguration.serverURL()
    }

    public static func catalog() throws -> DocumentCatalog {
        guard let service = try factory.withLock({ $0 })() else { throw ShortcutError.notSignedIn }
        return DocumentCatalog(service: service)
    }

    /// Runs `work` and rewrites whatever it throws into a message the person can read.
    public static func run<T>(_ work: (DocumentCatalog) async throws -> T) async throws -> T {
        do {
            return try await work(catalog())
        } catch {
            throw ShortcutError(mapping: error)
        }
    }

    @Sendable private static func signedInService() throws -> (any FolioService)? {
        guard try KeychainCredentialStore().load()?.isSignedIn == true else { return nil }
        return SharedConfiguration.makeService()
    }
}
