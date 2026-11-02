import FolioKit
import SwiftUI

/// What the menu bar (Mac) and the Command-key overlay (iPad) can ask the front window to do.
/// Keys match docs/shortcuts.md; change them there first.
struct InboxActions {
    var upload: (() -> Void)?
    /// Nil where the device has no scanner.
    var scan: (() -> Void)?
    var showInbox: () -> Void
    var showAll: () -> Void
    var find: () -> Void
    /// Nil while no document is selected.
    var deleteSelected: (() -> Void)?
}

extension FocusedValues {
    @Entry var inboxActions: InboxActions?
}

struct FolioCommands: Commands {
    @FocusedValue(\.inboxActions) private var actions

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Button("Upload…") { actions?.upload?() }
                .keyboardShortcut("o")
                .disabled(actions?.upload == nil)
            #if os(iOS)
                Button("Scan document") { actions?.scan?() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
                    .disabled(actions?.scan == nil)
            #endif
            Divider()
            Button("Delete document") { actions?.deleteSelected?() }
                .keyboardShortcut(.delete)
                .disabled(actions?.deleteSelected == nil)
        }
        CommandGroup(after: .sidebar) {
            Button("Inbox") { actions?.showInbox() }
                .keyboardShortcut("1")
                .disabled(actions == nil)
            Button("All documents") { actions?.showAll() }
                .keyboardShortcut("2")
                .disabled(actions == nil)
        }
        CommandGroup(after: .textEditing) {
            Button("Find…") { actions?.find() }
                .keyboardShortcut("f")
                .disabled(actions == nil)
        }
    }
}
