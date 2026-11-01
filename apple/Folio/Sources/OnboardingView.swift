import FolioKit
import SwiftUI

struct OnboardingView: View {
    @State private var onboarding: OnboardingViewModel

    init(model: AppModel) {
        _onboarding = State(
            initialValue: OnboardingViewModel(
                store: UserDefaultsServerConfigStore.shared(),
                makeService: AppModel.makeService,
                onConnected: model.connected
            )
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space6) {
            Text("Your paperwork, filed.")
                .font(FolioTheme.Typography.display)
                .foregroundStyle(Color.ink)
            Text("Enter the address of your Folio server to get started.")
                .font(FolioTheme.Typography.body)
                .foregroundStyle(Color.inkMuted)

            VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                Text("SERVER ADDRESS")
                    .font(FolioTheme.Typography.label)
                    .foregroundStyle(Color.inkMuted)
                TextField("folio.example.com", text: $onboarding.address)
                    .font(FolioTheme.Typography.meta)
                    .textFieldStyle(.plain)
                    .padding(FolioTheme.Spacing.space3)
                    .background(Color.paperSunken, in: .rect(cornerRadius: FolioTheme.Radius.radiusMd))
                    .overlay(
                        RoundedRectangle(cornerRadius: FolioTheme.Radius.radiusMd)
                            .stroke(Color.controlBorder, lineWidth: 1)
                    )
                    .autocorrectionDisabled()
                    .onSubmit { Task { await onboarding.connect() } }
                    #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                    #endif
            }

            if case let .failed(message) = onboarding.state {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(FolioTheme.Typography.bodySm)
                    .foregroundStyle(Color.danger)
                    .padding(FolioTheme.Spacing.space3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.dangerTint, in: .rect(cornerRadius: FolioTheme.Radius.radiusMd))
            }

            Button {
                Task { await onboarding.connect() }
            } label: {
                Text(onboarding.state == .checking ? "Connecting…" : "Connect")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(onboarding.state == .checking || onboarding.address.isEmpty)
        }
        .padding(FolioTheme.Spacing.space6)
        .frame(maxWidth: 440)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.paper)
    }
}
