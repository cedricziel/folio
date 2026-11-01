import AppIntents
import FolioKit

/// Pulls the intents defined in FolioKit into this app's metadata, so Shortcuts, Siri and Spotlight see them.
struct FolioAppIntents: AppIntentsPackage {
    static var includedPackages: [any AppIntentsPackage.Type] { [FolioIntentsPackage.self] }
}

struct FolioShortcuts: AppShortcutsProvider {
    static var shortcutTileColor: ShortcutTileColor { .teal }

    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: ScanDocumentIntent(),
            phrases: [
                "Scan a document with \(.applicationName)",
                "Scan with \(.applicationName)",
            ],
            shortTitle: "Scan a document", systemImageName: "doc.viewfinder")
        AppShortcut(
            intent: SearchDocumentsIntent(),
            phrases: [
                "Search \(.applicationName)",
                "Search my documents in \(.applicationName)",
            ],
            shortTitle: "Search documents", systemImageName: "magnifyingglass")
        AppShortcut(
            intent: GetLatestDocumentIntent(),
            phrases: [
                "Show my latest document in \(.applicationName)",
                "What is my latest \(.applicationName) document",
            ],
            shortTitle: "Latest document", systemImageName: "doc.text")
        AppShortcut(
            intent: FindDocumentsIntent(),
            phrases: ["Show \(\.$tag) documents in \(.applicationName)"],
            shortTitle: "Documents with a tag", systemImageName: "tag")
        AppShortcut(
            intent: UploadFileIntent(),
            phrases: ["Upload a file to \(.applicationName)"],
            shortTitle: "Upload a file", systemImageName: "tray.and.arrow.down")
    }
}
