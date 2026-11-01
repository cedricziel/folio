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
}
