import FileProvider

/// Registers Folio as a location in Files and Finder while a server is configured.
enum FileProviderDomain {
    private static var domain: NSFileProviderDomain {
        NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier("folio"), displayName: "Folio")
    }

    static func register() {
        NSFileProviderManager.add(domain) { _ in }
    }

    static func unregister() {
        NSFileProviderManager.remove(domain) { _ in }
    }

    /// Files and Finder don't poll the extension, so without this signal folders they already listed
    /// keep their contents until the system happens to enumerate them again.
    static func signalChanges() {
        NSFileProviderManager(for: domain)?.signalEnumerator(for: .workingSet) { _ in }
    }
}
