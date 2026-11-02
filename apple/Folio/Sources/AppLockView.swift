import FolioKit
import SwiftUI

struct AppLockView: View {
    let lock: AppLock

    var body: some View {
        ContentUnavailableView {
            Label("Folio is locked", systemImage: "lock.fill")
        } description: {
            Text(lock.problem ?? "Unlock with biometrics or your device credential to view your documents.")
        } actions: {
            Button("Unlock") { Task { await lock.unlock() } }
                .buttonStyle(.borderedProminent)
                .disabled(lock.isAuthenticating)
            if lock.isAuthenticating { ProgressView() }
        }
        .background(Color.paper)
        .navigationTitle("Folio")
    }
}

struct AppLockSettingsView: View {
    let lock: AppLock
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle(
                        "App lock",
                        isOn: Binding(
                            get: { lock.isEnabled },
                            set: { enabled in Task { await lock.setEnabled(enabled) } }
                        )
                    )
                    if lock.isEnabled {
                        Picker(
                            "Require unlock",
                            selection: Binding(
                                get: { lock.timeout },
                                set: { timeout in Task { await lock.setTimeout(timeout) } }
                            )
                        ) {
                            ForEach(AppLock.Timeout.allCases) { timeout in
                                Text(timeout.title).tag(timeout)
                            }
                        }
                    }
                } footer: {
                    Text(
                        "Use Face ID, Touch ID, or your device passcode or Mac password. The delay starts when you leave Folio. Folio always locks on launch and when your device locks."
                    )
                }
                Section {
                    Text(
                        "Locks the Folio app. Documents remain accessible through Files, Finder, and Shortcuts. Sharing to Folio continues to work."
                    )
                    .foregroundStyle(Color.inkMuted)
                }
                if let problem = lock.problem {
                    Text(problem).foregroundStyle(Color.danger)
                }
                if lock.isAuthenticating { ProgressView("Authenticating…") }
            }
            .formStyle(.grouped)
            .disabled(lock.isAuthenticating)
            .navigationTitle("App lock")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .padding(FolioTheme.Spacing.space4)
        #if os(macOS)
            .frame(minWidth: 400, idealWidth: 480, minHeight: 360, idealHeight: 420)
        #endif
    }
}

struct AppLockCommands: Commands {
    let lock: AppLock

    var body: some Commands {
        CommandGroup(after: .appSettings) {
            Button("Lock Folio now") { lock.lockNow() }
                .keyboardShortcut("l", modifiers: [.command, .shift])
                .disabled(!lock.isEnabled || lock.isLocked)
        }
    }
}
