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
    private let legacyKey = "folio.telemetry.installationID"

    private func defaults() throws -> (String, UserDefaults) {
        let suite = "folio-installation-test-\(UUID())"
        return (suite, try #require(UserDefaults(suiteName: suite)))
    }

    @Test func migratesLegacyUUIDToSharedKeychainWithoutChangingIdentity() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let legacy = UUID()
        preferences.set(legacy.uuidString, forKey: legacyKey)
        let store = TestInstallationIDStore()
        let identity = InstallationIdentity(defaults: preferences, store: store)
        #expect(identity.resolve() == legacy)
        #expect(try store.load() == legacy)
        #expect(preferences.string(forKey: legacyKey) == nil)
        #expect(InstallationIdentity(defaults: preferences, store: store).resolve() == legacy)
    }

    @Test func existingKeychainUUIDWinsOverLegacyPreferences() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let existing = UUID()
        preferences.set(UUID().uuidString, forKey: legacyKey)
        let store = TestInstallationIDStore(existing)
        #expect(InstallationIdentity(defaults: preferences, store: store).resolve() == existing)
        #expect(try store.load() == existing)
        #expect(preferences.string(forKey: legacyKey) == nil)
    }

    @Test func newInstallationPersistsBeforeExposingItsUUID() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let store = TestInstallationIDStore()
        let first = try #require(InstallationIdentity(defaults: preferences, store: store).resolve())
        #expect(try store.load() == first)
        #expect(InstallationIdentity(defaults: preferences, store: store).resolve() == first)
        #expect(preferences.string(forKey: legacyKey) == nil)
    }

    @Test func lockedKeychainDefersTelemetryAndPreservesLegacyMigration() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let legacy = UUID()
        preferences.set(legacy.uuidString, forKey: legacyKey)
        let store = TestInstallationIDStore()
        store.readUnavailable = true
        let identity = InstallationIdentity(defaults: preferences, store: store)
        #expect(identity.resolve() == nil)
        #expect(preferences.string(forKey: legacyKey) == legacy.uuidString)
        store.readUnavailable = false
        #expect(identity.resolve() == legacy)
        #expect(try store.load() == legacy)
    }

    @Test func failedMigrationWriteKeepsLegacyUUIDForRetry() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let legacy = UUID()
        preferences.set(legacy.uuidString, forKey: legacyKey)
        let store = TestInstallationIDStore()
        store.writeUnavailable = true
        let identity = InstallationIdentity(defaults: preferences, store: store)
        #expect(identity.resolve() == nil)
        #expect(try store.load() == nil)
        #expect(preferences.string(forKey: legacyKey) == legacy.uuidString)
        store.writeUnavailable = false
        #expect(identity.resolve() == legacy)
    }

    @Test func cachedIdentitySurvivesTemporaryKeychainLock() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let existing = UUID()
        let store = TestInstallationIDStore(existing)
        let identity = InstallationIdentity(defaults: preferences, store: store)
        #expect(identity.resolve() == existing)
        store.readUnavailable = true
        #expect(identity.resolve() == existing)
        #expect(InstallationIdentity(defaults: preferences, store: store).resolve() == nil)
    }

    @Test func concurrentProcessInsertionKeepsTheWinningKeychainUUID() throws {
        let (suite, preferences) = try defaults()
        defer { preferences.removePersistentDomain(forName: suite) }
        let store = TestInstallationIDStore()
        let winner = UUID()
        store.competingID = winner
        #expect(InstallationIdentity(defaults: preferences, store: store).resolve() == winner)
        #expect(try store.load() == winner)
    }
}
