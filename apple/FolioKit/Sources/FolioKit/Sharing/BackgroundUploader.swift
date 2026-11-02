import Foundation

/// A request plus the multipart body written to disk, which is what background sessions upload from.
public struct ShareUploadRequest {
    public let request: URLRequest
    public let bodyFile: URL

    public static func make(file: UploadFile, serverURL: URL, token: String?, directory: URL) throws
        -> ShareUploadRequest
    {
        let form = MultipartForm(file: file)
        let bodyFile = directory.appending(path: "upload-\(UUID().uuidString).multipart")
        try form.body.write(to: bodyFile)

        var request = URLRequest(url: serverURL.appending(path: "api/documents/upload"))
        request.httpMethod = "POST"
        request.setValue(form.contentType, forHTTPHeaderField: "Content-Type")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return ShareUploadRequest(request: request, bodyFile: bodyFile)
    }
}

public protocol BackgroundUploading: Sendable {
    func enqueue(_ file: UploadFile) throws
}

/// Uploads through a background `URLSession`, so large files finish after the share sheet has closed.
public final class BackgroundUploader: NSObject, BackgroundUploading, URLSessionTaskDelegate, @unchecked Sendable {
    private let serverURL: URL
    private let token: String?
    private let directory: URL
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.background(withIdentifier: identifier)
        configuration.sharedContainerIdentifier = SharedConfiguration.appGroup
        configuration.sessionSendsLaunchEvents = true
        return URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()

    private let identifier: String

    public init(serverURL: URL, token: String?, identifier: String, directory: URL? = nil) {
        self.serverURL = serverURL
        self.token = token
        self.identifier = identifier
        self.directory = directory ?? SharedConfiguration.container ?? FileManager.default.temporaryDirectory
        super.init()
    }

    public func enqueue(_ file: UploadFile) throws {
        let built = try ShareUploadRequest.make(
            file: file, serverURL: serverURL, token: token, directory: directory
        )
        let task = session.uploadTask(with: built.request, fromFile: built.bodyFile)
        task.taskDescription = built.bodyFile.path
        task.resume()
    }

    public func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        TelemetryRedirectGuard().urlSession(
            session, task: task, willPerformHTTPRedirection: response,
            newRequest: request, completionHandler: completionHandler)
    }

    public func urlSession(_: URLSession, task: URLSessionTask, didCompleteWithError _: (any Error)?) {
        if let path = task.taskDescription {
            try? FileManager.default.removeItem(atPath: path)
        }
    }
}
