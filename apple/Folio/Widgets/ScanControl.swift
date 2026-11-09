import AppIntents
import FolioKit
import SwiftUI
import WidgetKit

/// Control Center, Lock Screen and Action button entry point for scanning. The intent opens the app, which shows
/// the scanner only once App lock is satisfied and lets the review choose the destination vault.
@available(iOS 18.0, macOS 26.0, *)
struct ScanControl: ControlWidget {
    static let kind = "com.cedricziel.folio.scan"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind) {
            ControlWidgetButton(action: ScanDocumentIntent()) {
                Label("Scan to Folio", systemImage: "doc.viewfinder")
            }
        }
        .displayName("Scan to Folio")
        .description("Opens Folio's document scanner.")
    }
}
