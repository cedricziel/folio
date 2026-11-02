import AppIntents
import Foundation

/// What a failed intent tells the person running it. Plain sentences, no blame, per design/README.md.
public struct ShortcutError: Error, Equatable, CustomLocalizedStringResourceConvertible {
    public let message: String

    public init(_ message: String) {
        self.message = message
    }

    public var localizedStringResource: LocalizedStringResource {
        "\(message)"
    }

    public static let notSignedIn = ShortcutError("Folio isn't signed in. Open Folio, sign in, then try again.")

    public init(mapping error: any Error) {
        switch error {
        case let error as ShortcutError:
            self = error
        case ShortcutFailure.notSignedIn, FolioError.signInRequired:
            self = .notSignedIn
        case ShortcutFailure.noDocuments:
            self.init("There are no documents in Folio yet.")
        case ShortcutFailure.documentNotFound:
            self.init("That document is no longer in Folio.")
        case let ShortcutFailure.unreadableFile(name):
            self.init("Couldn't read \(name).")
        case FolioError.unreachable:
            self.init("Couldn't reach your Folio server. Check the connection and try again.")
        case let FolioError.server(_, message?):
            self.init(message)
        case let FolioError.server(status, nil):
            self.init("The server answered with status \(status).")
        case FolioError.unreadableResponse:
            self.init("Couldn't read the server's answer.")
        case FolioError.invalidServerURL:
            self.init("The server address in Folio isn't valid. Check it in the app.")
        case let FolioError.signInFailed(message):
            self.init(message)
        default:
            self.init(error.localizedDescription)
        }
    }
}
