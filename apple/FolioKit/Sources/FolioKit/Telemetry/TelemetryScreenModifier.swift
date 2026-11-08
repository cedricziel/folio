import SwiftUI

extension View {
    /// Records `screen` in telemetry when this view appears and whenever `screen` changes while it is visible.
    public func telemetryScreen(_ screen: TelemetryScreen) -> some View {
        onAppear { FolioTelemetry.shared.screen(screen) }
            .onChange(of: screen) { _, next in FolioTelemetry.shared.screen(next) }
    }
}
