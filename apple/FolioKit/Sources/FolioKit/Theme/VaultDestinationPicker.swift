import SwiftUI

public struct VaultDestinationPicker: View {
    @Bindable private var destination: VaultDestinationViewModel

    public init(destination: VaultDestinationViewModel) {
        self.destination = destination
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
            if destination.isReady {
                if destination.catalog.vaults.count > 1 {
                    Picker(
                        "Upload to",
                        selection: Binding(
                            get: { destination.selectedVaultID },
                            set: {
                                if let id = $0 {
                                    destination.selectVault(id)
                                }
                            }
                        )
                    ) {
                        ForEach(destination.catalog.vaults) { vault in
                            Text("\(vault.name) · \(vault.role.title)")
                                .tag(Optional(vault.id))
                                .disabled(!vault.role.canUpload)
                        }
                    }
                } else {
                    LabeledContent("Upload to", value: destination.title)
                }
                if !destination.canUpload {
                    Label("You have read-only access to this vault.", systemImage: "lock")
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.inkMuted)
                }
            } else if let error = destination.error {
                Text(error).foregroundStyle(Color.danger)
                Button("Try again") { Task { await destination.load() } }
            } else {
                ProgressView("Loading vaults…")
            }
        }
        .task { await destination.load() }
    }
}
