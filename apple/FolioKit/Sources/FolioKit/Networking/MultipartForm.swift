import Foundation

/// multipart/form-data body for `POST /api/documents/upload`: a `file` part and an optional `title`.
struct MultipartForm {
    let boundary = "folio-\(UUID().uuidString)"
    let body: Data

    var contentType: String {
        "multipart/form-data; boundary=\(boundary)"
    }

    init(file: UploadFile) {
        var body = Data()
        func append(_ string: String) {
            body.append(Data(string.utf8))
        }

        if let title = file.title {
            append("--\(boundary)\r\n")
            append("Content-Disposition: form-data; name=\"title\"\r\n\r\n\(title)\r\n")
        }
        append("--\(boundary)\r\n")
        append(
            "Content-Disposition: form-data; name=\"file\"; filename=\"\(Self.escape(file.filename))\"\r\n"
        )
        append("Content-Type: \(file.mimeType)\r\n\r\n")
        body.append(file.data)
        append("\r\n--\(boundary)--\r\n")
        self.body = body
    }

    private static func escape(_ filename: String) -> String {
        filename.replacingOccurrences(of: "\"", with: "%22")
            .replacingOccurrences(of: "\r", with: "").replacingOccurrences(of: "\n", with: "")
    }
}
