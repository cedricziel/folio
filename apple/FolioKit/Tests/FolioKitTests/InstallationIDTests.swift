import Foundation
import Testing

@testable import FolioKit

final class TestInstallationIDStore: InstallationIDStore, @unchecked Sendable {
    enum Failure: Error { case unavailable }
    private let lock = NSLock()
    var readUnavailable = false
    var writeUnavailable = false
    var competingID: UUID?
    private var value: UUID?

    init(_ value: UUID? = nil) { self.value = value }

    func load() throws -> UUID? {
        try lock.withLock {
            if readUnavailable { throw Failure.unavailable }
            return value
        }
    }

    func insertIfAbsent(_ candidate: UUID) throws -> UUID {
        try lock.withLock {
            if writeUnavailable { throw Failure.unavailable }
            if let competingID { value = competingID }
            if value == nil { value = candidate }
            return value!
        }
    }
}

struct InstallationIDTests {
    @Test func existingKeychainUUIDIsReused() throws {
        let existing = UUID()
        let store = TestInstallationIDStore(existing)
        #expect(InstallationIdentity(store: store).resolve() == existing)
        #expect(try store.load() == existing)
    }

    @Test func newInstallationPersistsBeforeExposingItsUUID() throws {
        let store = TestInstallationIDStore()
        let first = try #require(InstallationIdentity(store: store).resolve())
        #expect(try store.load() == first)
        #expect(InstallationIdentity(store: store).resolve() == first)
    }

    @Test func lockedKeychainDefersTelemetryUntilIdentityCanBeRead() throws {
        let existing = UUID()
        let store = TestInstallationIDStore(existing)
        store.readUnavailable = true
        let identity = InstallationIdentity(store: store)
        #expect(identity.resolve() == nil)
        store.readUnavailable = false
        #expect(identity.resolve() == existing)
        #expect(try store.load() == existing)
    }

    @Test func failedWriteDefersTelemetryUntilIdentityCanBePersisted() throws {
        let store = TestInstallationIDStore()
        store.writeUnavailable = true
        let identity = InstallationIdentity(store: store)
        #expect(identity.resolve() == nil)
        #expect(try store.load() == nil)
        store.writeUnavailable = false
        let persisted = try #require(identity.resolve())
        #expect(try store.load() == persisted)
    }

    @Test func cachedIdentitySurvivesTemporaryKeychainLock() throws {
        let existing = UUID()
        let store = TestInstallationIDStore(existing)
        let identity = InstallationIdentity(store: store)
        #expect(identity.resolve() == existing)
        store.readUnavailable = true
        #expect(identity.resolve() == existing)
        #expect(InstallationIdentity(store: store).resolve() == nil)
    }

    @Test func concurrentProcessInsertionKeepsTheWinningKeychainUUID() throws {
        let store = TestInstallationIDStore()
        let winner = UUID()
        store.competingID = winner
        #expect(InstallationIdentity(store: store).resolve() == winner)
        #expect(try store.load() == winner)
    }
}
