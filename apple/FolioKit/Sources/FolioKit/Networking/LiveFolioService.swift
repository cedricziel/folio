import FolioAPI
import Foundation
import ImageIO
import OpenAPIRuntime
import OpenAPIURLSession
import OpenTelemetryApi
import UniformTypeIdentifiers

/// Talks to a Folio server. JSON endpoints go through the generated client; file transfer uses
/// URLSession directly because the generated client cannot set per-part content types on uploads
/// or accept the server's file-specific download content types.
public struct LiveFolioService: FolioService, SourceActivityService, CustomMetadataService {
    private let baseURL: URL
    private let tokenProvider: any BearerTokenProvider
    private let session: URLSession
    private let downloadDirectory: URL?
    private let client: Client
    private let telemetry: FolioTelemetry?
    private let vaultID: UUID?

    public init(
        baseURL: URL,
        tokenProvider: any BearerTokenProvider = NoToken(),
        session: URLSession? = nil,
        telemetry: FolioTelemetry? = nil,
        downloadDirectory: URL? = nil,
        vaultID: UUID? = nil
    ) {
        let session =
            session ?? URLSession(configuration: .ephemeral, delegate: TelemetryRedirectGuard(), delegateQueue: nil)
        self.telemetry = telemetry
        self.baseURL = baseURL
        self.tokenProvider = tokenProvider
        self.session = session
        self.downloadDirectory = downloadDirectory
        self.vaultID = vaultID
        client = Client(
            serverURL: baseURL,
            configuration: .init(dateTranscoder: LenientDateTranscoder()),
            transport: URLSessionTransport(configuration: .init(session: session)),
            // Outermost first: the error mapper sees the response after the bearer middleware retried a 401.
            middlewares: [
                ErrorMappingMiddleware(), VaultScopeMiddleware(vaultID: vaultID),
                BearerAuthMiddleware(provider: tokenProvider),
            ]
        )
    }

    public func scoped(to vaultID: UUID) -> any FolioService {
        scopedClient(to: vaultID)
    }

    private func scopedClient(to vaultID: UUID) -> LiveFolioService {
        LiveFolioService(
            baseURL: baseURL, tokenProvider: tokenProvider, session: session, telemetry: telemetry,
            downloadDirectory: downloadDirectory, vaultID: vaultID
        )
    }

    public func sourceActivity(vaultID: UUID) async throws -> [SourceActivity] {
        try await sourceActivity(vaultID: vaultID, deleted: false)
    }

    public func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage {
        let scoped = LiveFolioService(
            baseURL: baseURL, tokenProvider: tokenProvider, session: session, telemetry: telemetry,
            downloadDirectory: downloadDirectory, vaultID: vaultID
        )
        return try await scoped.call("sources.outcomes") {
            let response = try await scoped.client.listSourceOutcomes(
                path: .init(id: sourceID.uuidString.lowercased()),
                query: .init(before: before, limit: 50)
            )
            let page = try response.ok.body.json
            guard try Self.uuid(page.vaultId) == vaultID, try Self.uuid(page.sourceId) == sourceID else {
                throw FolioError.unreadableResponse
            }
            let outcomes = try page.outcomes.map { row in
                guard try Self.uuid(row.vaultId) == vaultID, try Self.uuid(row.sourceId) == sourceID,
                    let result = SourceOutcome.Result(rawValue: row.outcome.rawValue)
                else { throw FolioError.unreadableResponse }
                return try SourceOutcome(
                    id: row.id, itemID: Self.uuid(row.itemId), attemptID: Self.uuid(row.attemptId),
                    occurredAt: row.occurredAt, outcome: result,
                    documentID: row.documentId.map(Self.uuid)
                )
            }
            return SourceOutcomePage(
                vaultID: vaultID, sourceID: sourceID, outcomes: outcomes, nextBefore: page.nextBefore
            )
        }
    }

    public func vaults() async throws -> VaultCatalog {
        try await call("vaults.list") {
            let request = URLRequest(url: endpoint("api/vaults"))
            let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
            try check(response, body: data)
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .custom { decoder in
                try LenientDateTranscoder().decode(decoder.singleValueContainer().decode(String.self))
            }
            guard let catalog = try? decoder.decode(VaultCatalog.self, from: data),
                catalog.defaultVaultID.map({ id in catalog.vaults.contains(where: { $0.id == id }) })
                    ?? catalog.vaults.isEmpty
            else { throw FolioError.unreadableResponse }
            return catalog
        }
    }

    func managementRequest(
        path: String, method: String = "GET", body: Data? = nil, vaultID: UUID? = nil
    ) async throws -> Data {
        let scoped = LiveFolioService(
            baseURL: baseURL, tokenProvider: tokenProvider, session: session, telemetry: telemetry,
            downloadDirectory: downloadDirectory, vaultID: vaultID
        )
        return try await scoped.call("management.request") {
            guard let url = URL(string: path, relativeTo: baseURL.appending(path: "/"))?.absoluteURL else {
                throw FolioError.invalidServerURL
            }
            var request = URLRequest(url: url)
            request.httpMethod = method
            request.httpBody = body
            request.setValue("application/json", forHTTPHeaderField: "Accept")
            if body != nil { request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
            let (data, response) = try await scoped.authorizedTransfer(request) { try await session.data(for: $0) }
            if let http = response as? HTTPURLResponse, http.statusCode == 403,
                let challenge = http.value(forHTTPHeaderField: "WWW-Authenticate"),
                challenge.contains("insufficient_scope"), challenge.contains("vaults:manage")
            {
                throw FolioError.server(
                    status: 403,
                    message:
                        "Sign out and sign in again to allow this app to manage vaults. If the server does not offer this permission, update it first."
                )
            }
            try scoped.check(response, body: data)
            return data
        }
    }

    public func savedViews(after: UUID?) async throws -> [SavedView] {
        guard let vaultID else { throw FolioError.server(status: 400, message: "Select a vault first.") }
        return try await call("saved_views.list") {
            try await client.listSavedViews(
                path: .init(vaultId: vaultID.uuidString.lowercased()),
                query: .init(after: after?.uuidString.lowercased(), limit: 50),
                headers: .init(xFolioSavedViewQueryVersion: 2)
            ).ok.body.json.map { try Self.savedView($0) }
        }
    }

    public func savedView(id: UUID) async throws -> SavedView {
        guard let vaultID else { throw FolioError.server(status: 400, message: "Select a vault first.") }
        return try await call("saved_views.get") {
            try await Self.savedView(
                client.getSavedView(
                    path: .init(vaultId: vaultID.uuidString.lowercased(), id: id.uuidString.lowercased())
                ).ok.body.json
            )
        }
    }

    public func saveView(_ draft: SavedViewDraft) async throws -> SavedView {
        guard let vaultID else { throw FolioError.server(status: 400, message: "Select a vault first.") }
        return try await call("saved_views.create") {
            try await Self.savedView(
                client.createSavedView(
                    path: .init(vaultId: vaultID.uuidString.lowercased()), body: .json(Self.savedViewInput(draft))
                ).created.body.json
            )
        }
    }

    public func updateSavedView(id: UUID, draft: SavedViewDraft) async throws -> SavedView {
        guard let vaultID else { throw FolioError.server(status: 400, message: "Select a vault first.") }
        return try await call("saved_views.update") {
            let input = Self.savedViewInput(draft)
            var patch = Components.Schemas.UpdateSavedView()
            patch.name = input.name
            patch.visibility = input.visibility
            patch.query = draft.replacesQuery ? input.query : nil
            patch.presentation = input.presentation
            return try await Self.savedView(
                client.updateSavedView(
                    path: .init(vaultId: vaultID.uuidString.lowercased(), id: id.uuidString.lowercased()),
                    body: .json(patch)
                ).ok.body.json
            )
        }
    }

    public func deleteSavedView(id: UUID) async throws {
        guard let vaultID else { throw FolioError.server(status: 400, message: "Select a vault first.") }
        try await call("saved_views.delete") {
            _ = try await client.deleteSavedView(
                path: .init(vaultId: vaultID.uuidString.lowercased(), id: id.uuidString.lowercased())
            ).noContent
        }
    }

    private static func savedViewInput(_ draft: SavedViewDraft) -> Components.Schemas.SavedViewInput {
        var filters = Components.Schemas.DocumentFilter()
        filters.correspondentId = draft.query.filter.correspondentID?.uuidString.lowercased()
        filters.documentTypeId = draft.query.filter.documentTypeID?.uuidString.lowercased()
        filters.tagIds = draft.query.filter.tagIDs.map { $0.uuidString.lowercased() }
        filters.searchQuery = draft.query.text
        filters.createdAfter = draft.query.filter.createdAfter
        filters.createdBefore = draft.query.filter.createdBefore
        filters.reviewed = draft.query.filter.reviewed
        filters.documentDateFrom = draft.query.filter.documentDateFrom?.rawValue
        filters.documentDateTo = draft.query.filter.documentDateTo?.rawValue
        filters.inboxOnly = draft.query.filter.inboxOnly ? true : nil
        var query = Components.Schemas.SavedViewQuery(version: draft.query.filter.inboxOnly ? 2 : 1)
        query.filters = filters
        query.sort = .init(rawValue: draft.query.sort.rawValue)
        var input = Components.Schemas.SavedViewInput(name: draft.name, query: query)
        input.visibility = .init(rawValue: draft.visibility.rawValue)
        input.presentation = .init(rawValue: draft.presentation.rawValue)
        return input
    }

    private static func savedView(_ value: Components.Schemas.SavedView) throws -> SavedView {
        let filters = value.query.filters ?? .init()
        let inboxOnly = filters.inboxOnly == true
        guard value.query.version == (inboxOnly ? 2 : 1),
            let visibility = SavedView.Visibility(rawValue: value.visibility.rawValue),
            let presentation = SavedView.Presentation(rawValue: value.presentation.rawValue)
        else { throw FolioError.unreadableResponse }
        let query = try DocumentBrowseQuery(
            text: filters.searchQuery ?? "",
            filter: .init(
                tagIDs: (filters.tagIds ?? []).map(Self.uuid),
                correspondentID: filters.correspondentId.map(Self.uuid),
                documentTypeID: filters.documentTypeId.map(Self.uuid),
                createdAfter: filters.createdAfter, createdBefore: filters.createdBefore,
                reviewed: filters.reviewed, documentDateFrom: Self.documentDate(filters.documentDateFrom),
                documentDateTo: Self.documentDate(filters.documentDateTo), inboxOnly: inboxOnly
            ), sort: .init(rawValue: value.query.sort?.rawValue ?? "newest") ?? .newest
        )
        return try SavedView(
            id: uuid(value.id), vaultID: uuid(value.vaultId), ownerID: value.ownerId, name: value.name,
            visibility: visibility, query: query, presentation: presentation,
            createdAt: value.createdAt, updatedAt: value.updatedAt,
            createdBy: value.createdBy, updatedBy: value.updatedBy
        )
    }

    public func entryPage(documentID: UUID, kind: DocumentEntryKind, cursor: String?) async throws -> DocumentEntryPage
    {
        try await call("document_entries.list") {
            let page = try await client.listDocumentEntries(
                path: .init(id: documentID.uuidString.lowercased()),
                query: .init(kind: kind.apiValue, limit: 50, cursor: cursor)
            ).ok.body.json
            return try DocumentEntryPage(
                entries: page.entries.map(DocumentEntry.init), nextCursor: page.nextCursor, canCreate: page.canCreate
            )
        }
    }

    public func createEntry(documentID: UUID, kind: DocumentEntryKind, body: String, key: UUID) async throws
        -> DocumentEntry
    {
        try await call("document_entries.create") {
            try await DocumentEntry(
                client.createDocumentEntry(
                    path: .init(id: documentID.uuidString.lowercased()),
                    headers: .init(idempotencyKey: key.uuidString.lowercased()),
                    body: .json(.init(body: body, kind: kind.apiValue))
                ).created.body.json
            )
        }
    }

    public func updateEntry(documentID: UUID, entryID: UUID, body: String) async throws -> DocumentEntry {
        try await call("document_entries.update") {
            try await DocumentEntry(
                client.updateDocumentEntry(
                    path: .init(id: documentID.uuidString.lowercased(), entryId: entryID.uuidString.lowercased()),
                    body: .json(.init(body: body))
                ).ok.body.json
            )
        }
    }

    public func deleteEntry(documentID: UUID, entryID: UUID) async throws {
        try await call("document_entries.delete") {
            _ = try await client.deleteDocumentEntry(
                path: .init(id: documentID.uuidString.lowercased(), entryId: entryID.uuidString.lowercased())
            ).noContent
        }
    }

    public func ping() async throws {
        _ = try await tags()
    }

    public func documentPage(query: DocumentBrowseQuery, cursor: String?) async throws -> DocumentPage {
        try await call("documents.browse", attributes: query.telemetryAttributes(cursor: cursor)) {
            let filter = query.filter
            let input = Operations.BrowseDocuments.Input.Query(
                inboxOnly: filter.inboxOnly ? true : nil,
                q: query.text.isEmpty ? nil : query.text,
                limit: query.limit, sort: query.sort.rawValue, cursor: cursor,
                reviewed: filter.reviewed, documentDateFrom: filter.documentDateFrom?.rawValue,
                documentDateTo: filter.documentDateTo?.rawValue,
                correspondentId: filter.correspondentID?.uuidString.lowercased(),
                documentTypeId: filter.documentTypeID?.uuidString.lowercased(),
                tagId: filter.tagIDs.isEmpty ? nil : filter.tagIDs.map { $0.uuidString.lowercased() },
                createdAfter: filter.createdAfter, createdBefore: filter.createdBefore
            )
            let page = try await client.browseDocuments(query: input).ok.body.json
            try requireReviewFilters(filter, supported: page.supportsDocumentReviewFilters)
            return try DocumentPage(
                vaultID: Self.uuid(page.vaultId), documents: page.documents.map { try Document($0) },
                nextCursor: page.nextCursor, supportsReviewFilters: page.supportsDocumentReviewFilters == true
            )
        }
    }

    public func document(id: Document.ID) async throws -> Document {
        try await call("documents.get") {
            try await Document(client.getDocument(path: .init(id: id.uuidString.lowercased())).ok.body.json)
        }
    }

    public func setDocumentReview(_ reviewed: Bool, for id: Document.ID) async throws -> Document {
        try await call("documents.review") {
            try await Document(
                client.setDocumentReview(
                    path: .init(id: id.uuidString.lowercased()), body: .json(.init(reviewed: reviewed))
                ).ok.body.json
            )
        }
    }

    public func setDocumentDate(_ date: DocumentDate?, for id: Document.ID) async throws -> Document {
        try await call("documents.date") {
            try await Document(
                client.updateDocumentDate(
                    path: .init(id: id.uuidString.lowercased()),
                    body: .json(.init(documentDate: .some(date?.rawValue)))
                ).ok.body.json
            )
        }
    }

    public func documents(matching query: String) async throws -> [Document] {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return try await call("documents.list") {
            if trimmed.isEmpty {
                return try await client.listDocuments().ok.body.json.documents.map { try Document($0) }
            }
            let results = try await client.searchDocuments(query: .init(q: trimmed, limit: 50))
            return try results.ok.body.json.results.map { try Document($0.document) }
        }
    }

    public func documents(filter: DocumentFilter) async throws -> [Document] {
        try await call("documents.filter") {
            let query = Operations.ListDocuments.Input.Query(
                reviewed: filter.reviewed, documentDateFrom: filter.documentDateFrom?.rawValue,
                documentDateTo: filter.documentDateTo?.rawValue,
                correspondentId: filter.correspondentID?.uuidString.lowercased(),
                documentTypeId: filter.documentTypeID?.uuidString.lowercased(),
                tagId: filter.tagIDs.isEmpty ? nil : filter.tagIDs.map { $0.uuidString.lowercased() },
                createdAfter: filter.createdAfter, createdBefore: filter.createdBefore
            )
            let response = try await client.listDocuments(query: query).ok.body.json
            try requireReviewFilters(filter, supported: response.supportsDocumentReviewFilters)
            return try response.documents.map { try Document($0) }
        }
    }

    /// Servers without the IDs route answer 404, or 400 where `ids` is read as a document ID; a proxy or an older
    /// token policy may answer 403 or 405. All of those fall back to the full list.
    public func documentIDs(filter: DocumentFilter) async throws -> [Document.ID] {
        do {
            return try await call("documents.ids") {
                let query = Operations.ListDocumentIds.Input.Query(
                    reviewed: filter.reviewed, documentDateFrom: filter.documentDateFrom?.rawValue,
                    documentDateTo: filter.documentDateTo?.rawValue,
                    correspondentId: filter.correspondentID?.uuidString.lowercased(),
                    documentTypeId: filter.documentTypeID?.uuidString.lowercased(),
                    tagId: filter.tagIDs.isEmpty ? nil : filter.tagIDs.map { $0.uuidString.lowercased() },
                    createdAfter: filter.createdAfter, createdBefore: filter.createdBefore
                )
                return try await client.listDocumentIds(query: query).ok.body.json.ids.map { try Self.uuid($0) }
            }
        } catch let FolioError.server(status, _) where [400, 403, 404, 405].contains(status) {
            return try await documents(filter: filter).map(\.id)
        }
    }

    private func requireReviewFilters(_ filter: DocumentFilter, supported: Bool?) throws {
        if filter.requiresReviewSupport, supported != true {
            throw FolioError.server(
                status: 501, message: "This server does not support review and document date filters."
            )
        }
    }

    /// The server only offers this as a form post that redirects back to the document page.
    public func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        try await call("documents.addTag") { try await addTagRequest(tagID, to: documentID) }
    }

    private func addTagRequest(_ tagID: Tag.ID, to documentID: Document.ID) async throws {
        var request = URLRequest(url: endpoint("api/documents/\(documentID.uuidString.lowercased())/tags"))
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data("tag_id=\(tagID.uuidString.lowercased())".utf8)
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
    }

    /// Also a form post, with the tag in the path.
    public func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        try await call("documents.removeTag") { try await removeTagRequest(tagID, from: documentID) }
    }

    private func removeTagRequest(_ tagID: Tag.ID, from documentID: Document.ID) async throws {
        var request = URLRequest(
            url: endpoint(
                "api/documents/\(documentID.uuidString.lowercased())/tags/\(tagID.uuidString.lowercased())"
            )
        )
        request.httpMethod = "POST"
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
    }

    /// Form posts like the tag routes; an empty value clears the field.
    public func setCorrespondent(_ correspondentID: Correspondent.ID?, for documentID: Document.ID) async throws {
        try await call("documents.correspondent") {
            try await classificationRequest(
                documentID, path: "correspondent", field: "correspondent_id", value: correspondentID)
        }
    }

    public func setDocumentType(_ documentTypeID: DocumentType.ID?, for documentID: Document.ID) async throws {
        try await call("documents.documentType") {
            try await classificationRequest(
                documentID, path: "document_type", field: "document_type_id", value: documentTypeID)
        }
    }

    private func classificationRequest(
        _ documentID: Document.ID, path: String, field: String, value: UUID?
    ) async throws {
        var request = URLRequest(url: endpoint("api/documents/\(documentID.uuidString.lowercased())/\(path)"))
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data("\(field)=\(value?.uuidString.lowercased() ?? "")".utf8)
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
    }

    public func deleteDocument(id: Document.ID) async throws {
        try await call("documents.delete") {
            _ = try await client.deleteDocument(path: .init(id: id.uuidString.lowercased())).noContent
        }
    }

    public func restoreDocument(id: Document.ID) async throws -> Document {
        try await call("documents.restore") {
            try await Document(client.restoreDocument(path: .init(id: id.uuidString.lowercased())).ok.body.json)
        }
    }

    public func taxonomy(_ kind: TaxonomyKind, deleted: Bool) async throws -> [TaxonomyEntry] {
        try await call("taxonomy.list") {
            let path = "api/\(kind.rawValue)" + (deleted ? "/trash" : "")
            let data = try await taxonomyRequest(path: path, method: "GET")
            let transcoder = LenientDateTranscoder()
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .custom { decoder in
                try transcoder.decode(decoder.singleValueContainer().decode(String.self))
            }
            do {
                return try decoder.decode([TaxonomyEntry].self, from: data)
            } catch is DecodingError {
                throw FolioError.unreadableResponse
            }
        }
    }

    public func saveTaxonomy(_ kind: TaxonomyKind, id: UUID?, name: String) async throws {
        try await call("taxonomy.save") {
            let path = "api/\(kind.rawValue)" + (id.map { "/" + $0.uuidString.lowercased() } ?? "")
            _ = try await taxonomyRequest(path: path, method: id == nil ? "POST" : "PUT", body: ["name": name])
        }
    }

    public func deleteTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        try await call("taxonomy.delete") {
            _ = try await taxonomyRequest(path: "api/\(kind.rawValue)/\(id.uuidString.lowercased())", method: "DELETE")
        }
    }

    public func restoreTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        try await call("taxonomy.restore") {
            _ = try await taxonomyRequest(
                path: "api/\(kind.rawValue)/\(id.uuidString.lowercased())/restore", method: "POST")
        }
    }

    private func taxonomyRequest(path: String, method: String, body: [String: String]? = nil) async throws -> Data {
        var request = URLRequest(url: endpoint(path))
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body {
            request.httpBody = try JSONEncoder().encode(body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
        try check(response, body: data)
        return data
    }

    public func tags() async throws -> [Tag] {
        try await call("tags.list") {
            try await client.listTags().ok.body.json.map {
                try Tag(
                    id: Self.uuid($0.id), name: $0.name, color: $0.color, deletedAt: $0.deletedAt,
                    createdAt: $0.createdAt)
            }
        }
    }

    public func correspondents() async throws -> [Correspondent] {
        try await call("correspondents.list") {
            try await client.listCorrespondents().ok.body.json.map {
                try Correspondent(id: Self.uuid($0.id), name: $0.name, createdAt: $0.createdAt)
            }
        }
    }

    public func documentTypes() async throws -> [DocumentType] {
        try await call("document_types.list") {
            try await client.listDocumentTypes().ok.body.json.map {
                try DocumentType(id: Self.uuid($0.id), name: $0.name, createdAt: $0.createdAt)
            }
        }
    }

    public func thumbnail(id: Document.ID) async throws -> Data? {
        try await call("documents.thumbnail") {
            var request = URLRequest(url: endpoint("api/documents/\(id.uuidString.lowercased())/thumbnail"))
            request.cachePolicy = .reloadIgnoringLocalCacheData
            request.setValue("image/jpeg", forHTTPHeaderField: "Accept")
            let (bytes, response) = try await authorizedTransfer(request) { request in
                let result: (URLSession.AsyncBytes, URLResponse)
                do {
                    result = try await session.bytes(for: request)
                } catch let error as URLError where error.code == .cancelled {
                    throw CancellationError()
                }
                if (result.1 as? HTTPURLResponse)?.statusCode == 401 {
                    result.0.task.cancel()
                }
                return result
            }
            defer { bytes.task.cancel() }
            try Task.checkCancellation()
            if (response as? HTTPURLResponse)?.statusCode == 404 {
                return nil
            }
            try check(response, body: nil)
            guard response.mimeType == "image/jpeg", response.expectedContentLength <= 2_097_152 else {
                throw FolioError.unreadableResponse
            }
            var data = Data()
            for try await byte in bytes {
                try Task.checkCancellation()
                guard data.count < 2_097_152 else { throw FolioError.unreadableResponse }
                data.append(byte)
            }
            guard let source = CGImageSourceCreateWithData(data as CFData, nil),
                CGImageSourceGetType(source) as String? == UTType.jpeg.identifier,
                let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
                let width = properties[kCGImagePropertyPixelWidth] as? Int,
                let height = properties[kCGImagePropertyPixelHeight] as? Int,
                width > 0, height > 0, width <= 4_000_000 / height,
                CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCache: false] as CFDictionary) != nil
            else { throw FolioError.unreadableResponse }
            return data
        }
    }

    public func downloadFile(for document: Document) async throws -> URL {
        try await call("documents.downloadFile") { try await downloadFileRequest(for: document) }
    }

    private func downloadFileRequest(for document: Document) async throws -> URL {
        let request = URLRequest(url: endpoint("api/documents/\(document.id.uuidString.lowercased())/download"))
        let (temporary, response) = try await authorizedTransfer(request) { try await session.download(for: $0) }
        defer { try? FileManager.default.removeItem(at: temporary) }
        try check(response, body: Self.prefix(of: temporary, upTo: 65_536))

        let filename = Self.safeFilename(document.originalFilename, mimeType: document.mimeType)
        let directory =
            downloadDirectory
            ?? FileManager.default.temporaryDirectory
            .appending(path: "folio-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let ext = (filename as NSString).pathExtension
        let suffix = !ext.isEmpty && ext.utf8.count <= 32 ? "." + ext : ""
        let destination = directory.appending(
            path: downloadDirectory == nil ? filename : UUID().uuidString + suffix
        )
        do {
            try FileManager.default.moveItem(at: temporary, to: destination)
        } catch {
            if downloadDirectory == nil {
                try? FileManager.default.removeItem(at: directory)
            }
            throw error
        }
        return destination
    }

    public func upload(_ file: UploadFile) async throws -> Document {
        try await call("documents.upload") { try await uploadRequest(file, idempotencyKey: nil) }
    }

    public func upload(_ file: UploadFile, idempotencyKey: UUID) async throws -> Document {
        try await call("documents.upload") { try await uploadRequest(file, idempotencyKey: idempotencyKey) }
    }

    private func uploadRequest(_ file: UploadFile, idempotencyKey: UUID?) async throws -> Document {
        var request = URLRequest(url: endpoint("api/documents/upload"))
        request.httpMethod = "POST"
        if let idempotencyKey {
            request.setValue(idempotencyKey.uuidString.lowercased(), forHTTPHeaderField: "Idempotency-Key")
        }
        let form = MultipartForm(file: file)
        request.setValue(form.contentType, forHTTPHeaderField: "Content-Type")

        let (data, response) = try await authorizedTransfer(request) {
            try await session.upload(for: $0, from: form.body)
        }
        if idempotencyKey != nil, let http = response as? HTTPURLResponse,
            let conflict = KeyedUploadConflict(response: http, body: data)
        {
            throw conflict
        }
        try check(response, body: data)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let string = try decoder.singleValueContainer().decode(String.self)
            return try LenientDateTranscoder().decode(string)
        }
        do {
            return try Document(decoder.decode(Components.Schemas.Document.self, from: data))
        } catch {
            throw FolioError.unreadableResponse
        }
    }

    /// The server controls the filename, so keep only its last path component and never a traversal.
    static func safeFilename(_ name: String?, mimeType: String?) -> String {
        let last = name.map { ($0 as NSString).lastPathComponent } ?? ""
        if !["", ".", "..", "/"].contains(last) {
            return last
        }
        let ext = mimeType.flatMap { UTType(mimeType: $0)?.preferredFilenameExtension }
        return ext.map { "document.\($0)" } ?? "document"
    }

    fileprivate static func documentDate(_ value: String?) throws -> DocumentDate? {
        guard let value else { return nil }
        guard let date = DocumentDate(rawValue: value) else { throw FolioError.unreadableResponse }
        return date
    }

    fileprivate static func uuid(_ string: String) throws -> UUID {
        guard let id = UUID(uuidString: string) else { throw FolioError.unreadableResponse }
        return id
    }

    private func endpoint(_ path: String) -> URL {
        baseURL.appending(path: path)
    }

    /// Sends a request with the bearer token; if the server rejects it, refreshes the token and sends it once more.
    private func authorizedTransfer<T>(
        _ request: URLRequest, _ perform: (URLRequest) async throws -> (T, URLResponse)
    ) async throws -> (T, URLResponse) {
        func signed(_ token: String?) -> URLRequest {
            var copy = request
            if let vaultID {
                copy.setValue(vaultID.uuidString.lowercased(), forHTTPHeaderField: "X-Folio-Vault")
            }
            if let token {
                copy.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            }
            return copy
        }
        let token = try await tokenProvider.token()
        let first = try await perform(signed(token))
        guard (first.1 as? HTTPURLResponse)?.statusCode == 401, let token,
            let fresh = try await tokenProvider.refresh(rejecting: token)
        else { return first }
        return try await perform(signed(fresh))
    }

    static func prefix(of file: URL, upTo limit: Int) -> Data? {
        guard let handle = try? FileHandle(forReadingFrom: file) else { return nil }
        defer { try? handle.close() }
        return try? handle.read(upToCount: limit)
    }

    private func check(_ response: URLResponse, body: @autoclosure () -> Data?) throws {
        guard let http = response as? HTTPURLResponse else { throw FolioError.unreadableResponse }
        if http.statusCode == 401 {
            throw FolioError.signInRequired
        }
        guard (200..<300).contains(http.statusCode) else {
            throw FolioError.server(status: http.statusCode, message: body().flatMap(ErrorMappingMiddleware.message))
        }
    }

    private func call<T>(
        _ name: String, attributes: [String: AttributeValue] = [:], _ work: () async throws -> T
    ) async throws -> T {
        // Telemetry sees the transport error before it is mapped, so a span can tell a timeout from a cancel.
        if let telemetry {
            return try await mappedCall { try await telemetry.operation(name, attributes: attributes, work) }
        }
        return try await mappedCall(work)
    }

    private func mappedCall<T>(_ work: () async throws -> T) async throws -> T {
        do {
            return try await work()
        } catch let error as FolioError {
            throw error
        } catch let error as URLError {
            throw FolioError(transport: error)
        } catch let error as ClientError {
            switch error.underlyingError {
            case let folio as FolioError: throw folio
            case let credentials as CredentialStoreError: throw credentials
            case let url as URLError: throw FolioError(transport: url)
            default: throw FolioError.unreadableResponse
            }
        }
    }
}

extension Document {
    init(_ raw: Components.Schemas.Document) throws {
        try self.init(
            id: LiveFolioService.uuid(raw.id), title: raw.title, content: raw.content,
            originalFilename: raw.originalFilename, mimeType: raw.mimeType,
            correspondentID: raw.correspondentId.map(LiveFolioService.uuid),
            documentTypeID: raw.documentTypeId.map(LiveFolioService.uuid),
            pageCount: raw.pageCount.map(Int.init),
            ocrStatus: .init(raw.ocrStatus), embedStatus: .init(raw.embedStatus),
            documentDate: LiveFolioService.documentDate(raw.documentDate),
            reviewedAt: raw.reviewedAt, reviewedBy: raw.reviewedBy, reviewOrigin: raw.reviewOrigin,
            createdAt: raw.createdAt, updatedAt: raw.updatedAt,
            createdBy: raw.createdBy, updatedBy: raw.updatedBy, deletedAt: raw.deletedAt,
            fileSize: raw.fileSize
        )
    }
}

extension Document.ProcessingStatus {
    init(_ raw: Components.Schemas.ProcessingStatus) {
        switch raw {
        case .pending: self = .pending
        case .running: self = .running
        case .done: self = .done
        case .failed: self = .failed
        case .skipped: self = .skipped
        }
    }
}

extension Document {
    /// The list and get endpoints add the document's tags to the plain document.
    init(_ raw: Components.Schemas.DocumentView) throws {
        try self.init(raw.value1)
        canEditDocumentDate = raw.value2.canEditDocumentDate ?? false
        canReviewDocument = raw.value2.canReviewDocument ?? false
        reviewed = raw.value2.reviewed
        processingStages = try (raw.value2.processingStages ?? []).map { stage in
            guard let kind = ProcessingStageSummary.Stage(rawValue: stage.stage.rawValue),
                let state = ProcessingStageSummary.State(rawValue: stage.state.rawValue)
            else { throw FolioError.unreadableResponse }
            return try ProcessingStageSummary(
                runID: LiveFolioService.uuid(stage.runId),
                revisionID: stage.revisionId.map(LiveFolioService.uuid),
                stage: kind, attempt: Int(stage.attempt), state: state,
                reason: stage.reasonCode.flatMap { ProcessingStageSummary.Reason(rawValue: $0) },
                retryAt: stage.retryAt, hasText: stage.hasText
            )
        }
        tags = try raw.value2.tags.map {
            try Tag(id: LiveFolioService.uuid($0.id), name: $0.name, color: $0.color, deletedAt: $0.deletedAt)
        }
    }
}

private extension DocumentEntryKind {
    var apiValue: Components.Schemas.EntryKind {
        switch self {
        case .note: .note
        case .comment: .comment
        }
    }
}

private extension DocumentEntry {
    init(_ raw: Components.Schemas.EntryView) throws {
        let entry = raw.value1
        try self.init(
            id: LiveFolioService.uuid(entry.id), documentID: LiveFolioService.uuid(entry.documentId),
            vaultID: LiveFolioService.uuid(entry.vaultId), kind: entry.kind == .note ? .note : .comment,
            body: entry.body, createdAt: entry.createdAt, updatedAt: entry.updatedAt,
            createdBy: entry.createdBy, canEdit: raw.value2.canEdit, canDelete: raw.value2.canDelete
        )
    }
}

extension LiveFolioService: DeviceRegistrationService {
    public func registerDevice(installationID: String, _ registration: DeviceRegistration) async throws
        -> DeviceRegistrationResult
    {
        try await call("devices.register") {
            var request = URLRequest(url: endpoint("api/devices/\(installationID)"))
            request.httpMethod = "PUT"
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONEncoder().encode(registration)
            let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
            try check(response, body: data)
            let wire = try JSONDecoder().decode(DeviceRegistrationWire.self, from: data)
            return DeviceRegistrationResult(preferences: wire.device.preferences, pushAvailable: wire.pushAvailable)
        }
    }

    public func deleteDevice(installationID: String) async throws {
        try await call("devices.delete") {
            var request = URLRequest(url: endpoint("api/devices/\(installationID)"))
            request.httpMethod = "DELETE"
            let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
            try check(response, body: data)
        }
    }

    public func sendTestNotification(installationID: String) async throws {
        try await call("devices.test") {
            var request = URLRequest(url: endpoint("api/devices/\(installationID)/test"))
            request.httpMethod = "POST"
            let (data, response) = try await authorizedTransfer(request) { try await session.data(for: $0) }
            try check(response, body: data)
        }
    }
}

extension LiveFolioService: WorkflowTriggerEditingService {
    public func workflowTriggerChoices(vaultID: UUID) async throws -> [WorkflowTriggerChoice] {
        let scoped = scopedClient(to: vaultID)
        return try await scoped.call("workflows.list_triggers") {
            let definitions = try await scoped.client.listWorkflowTriggers().ok.body.json
            return try definitions.map { definition in
                guard let target = WorkflowTriggerChoice.Target(rawValue: definition.target.rawValue) else {
                    throw FolioError.unreadableResponse
                }
                return WorkflowTriggerChoice(
                    id: definition.eventType, title: definition.label, target: target,
                    conditionFields: definition.conditionFields.map { field in
                        .init(id: field.key, title: field.label, valueType: field.valueType.rawValue)
                    }
                )
            }
        }
    }

    public func workflowConfiguration(vaultID: UUID, workflowID: UUID) async throws
        -> WorkflowConfigurationSummary
    {
        let scoped = scopedClient(to: vaultID)
        let id = workflowID.uuidString.lowercased()
        return try await scoped.call("workflows.get") {
            let workflow = try await Self.workflowSummary(scoped.client.getWorkflow(path: .init(id: id)).ok.body.json)
            guard workflow.id == workflowID, workflow.vaultID == vaultID, workflow.deletedAt == nil else {
                throw FolioError.unreadableResponse
            }
            return workflow
        }
    }

    public func changeWorkflowTrigger(_ workflow: WorkflowConfigurationSummary, to triggerID: String) async throws
        -> WorkflowConfigurationSummary
    {
        guard workflow.deletedAt == nil, let revision = workflow.revision,
            let trigger = Components.Schemas.WorkflowTrigger(rawValue: triggerID)
        else { throw FolioError.unreadableResponse }
        let scoped = scopedClient(to: workflow.vaultID)
        let id = workflow.id.uuidString.lowercased()
        return try await scoped.call("workflows.change_trigger") {
            let updated = try await scoped.client.updateWorkflowTrigger(
                path: .init(id: id),
                body: .json(.init(expectedRevision: revision, trigger: trigger))
            ).ok.body.json
            let result = try Self.workflowSummary(updated)
            guard result.vaultID == workflow.vaultID, result.id == workflow.id,
                result.deletedAt == nil, result.trigger == triggerID
            else {
                throw FolioError.unreadableResponse
            }
            return result
        }
    }

    private static func workflowSummary(_ value: Components.Schemas.Workflow) throws -> WorkflowConfigurationSummary {
        .init(
            id: try uuid(value.id), vaultID: try uuid(value.vaultId), name: value.name, enabled: value.enabled,
            trigger: value.trigger.rawValue, createdAt: value.createdAt, updatedAt: value.updatedAt,
            deletedAt: value.deletedAt, createdBy: value.createdBy, updatedBy: value.updatedBy, revision: value.revision
        )
    }
}

extension LiveFolioService {
    private var metadataVault: String {
        get throws {
            guard let vaultID else { throw FolioError.unreadableResponse }
            return vaultID.uuidString.lowercased()
        }
    }

    public func customFields(after: UUID?) async throws -> [CustomFieldDefinition] {
        try await call("metadata_fields.list") {
            let vault = try metadataVault
            let fields = try await client.listMetadataFields(
                path: .init(vaultId: vault),
                query: .init(includeDeleted: true, after: after?.uuidString.lowercased(), limit: 50)
            ).ok.body.json
            return try fields.map { try customField($0) }
        }
    }

    public func saveCustomField(id: UUID?, draft: CustomFieldDraft) async throws -> CustomFieldDefinition {
        try await call("metadata_fields.save") {
            let vault = try metadataVault
            guard let kind = Components.Schemas.MetadataKind(rawValue: draft.kind.rawValue) else {
                throw FolioError.unreadableResponse
            }
            let input = Components.Schemas.MetadataFieldInput(
                kind: kind, name: draft.name, required: draft.required, unique: draft.unique
            )
            if let id {
                return try await customField(
                    client.updateMetadataField(
                        path: .init(vaultId: vault, fieldId: id.uuidString.lowercased()), body: .json(input)
                    ).ok.body.json
                )
            }
            return try await customField(
                client.createMetadataField(
                    path: .init(vaultId: vault), body: .json(input)
                ).created.body.json)
        }
    }

    public func setCustomFieldRetired(id: UUID, retired: Bool) async throws {
        try await call("metadata_fields.lifecycle") {
            let vault = try metadataVault
            let field = id.uuidString.lowercased()
            if retired {
                _ = try await client.deleteMetadataField(path: .init(vaultId: vault, fieldId: field)).noContent
            } else {
                _ = try await client.restoreMetadataField(path: .init(vaultId: vault, fieldId: field)).ok.body.json
            }
        }
    }

    public func customOption(fieldID: UUID, id: UUID) async throws -> CustomFieldOption {
        try await call("metadata_options.get") {
            let option = try await client.getMetadataOption(
                path: .init(
                    vaultId: metadataVault, fieldId: fieldID.uuidString.lowercased(),
                    optionId: id.uuidString.lowercased()
                )
            ).ok.body.json
            return try customOption(option, fieldID: fieldID)
        }
    }

    public func customOptions(fieldID: UUID, after: UUID?) async throws -> [CustomFieldOption] {
        try await call("metadata_options.list") {
            let options = try await client.listMetadataOptions(
                path: .init(vaultId: metadataVault, fieldId: fieldID.uuidString.lowercased()),
                query: .init(includeDeleted: true, after: after?.uuidString.lowercased(), limit: 50)
            ).ok.body.json
            return try options.map { try customOption($0, fieldID: fieldID) }
        }
    }

    public func saveCustomOption(
        fieldID: UUID, id: UUID?, label: String, position: UInt32
    ) async throws -> CustomFieldOption {
        try await call("metadata_options.save") {
            let vault = try metadataVault
            let field = fieldID.uuidString.lowercased()
            let input = Components.Schemas.MetadataOptionInput(label: label, position: Int64(position))
            let option: Components.Schemas.MetadataOption
            if let id {
                option = try await client.updateMetadataOption(
                    path: .init(vaultId: vault, fieldId: field, optionId: id.uuidString.lowercased()),
                    body: .json(input)
                ).ok.body.json
            } else {
                option = try await client.createMetadataOption(
                    path: .init(vaultId: vault, fieldId: field), body: .json(input)
                ).created.body.json
            }
            return try customOption(option, fieldID: fieldID)
        }
    }

    public func setCustomOptionRetired(fieldID: UUID, id: UUID, retired: Bool) async throws {
        try await call("metadata_options.lifecycle") {
            let vault = try metadataVault
            let field = fieldID.uuidString.lowercased()
            let option = id.uuidString.lowercased()
            if retired {
                _ = try await client.deleteMetadataOption(
                    path: .init(vaultId: vault, fieldId: field, optionId: option)
                ).noContent
            } else {
                _ = try await client.restoreMetadataOption(
                    path: .init(vaultId: vault, fieldId: field, optionId: option)
                ).ok.body.json
            }
        }
    }

    public func customMetadata(documentID: UUID) async throws -> CustomMetadataSnapshot {
        try await call("document_metadata.get") {
            let snapshot = try await client.getDocumentMetadata(
                path: .init(vaultId: metadataVault, documentId: documentID.uuidString.lowercased())
            ).ok.body.json
            return try Self.customMetadata(snapshot, documentID: documentID)
        }
    }

    public func setCustomValue(
        documentID: UUID, revision: UUID?, fieldID: UUID, value: CustomFieldValue?
    ) async throws -> CustomMetadataSnapshot {
        try await call("document_metadata.patch") {
            let null = try OpenAPIValueContainer(unvalidatedValue: nil)
            let snapshot = try await client.patchDocumentMetadata(
                path: .init(vaultId: metadataVault, documentId: documentID.uuidString.lowercased()),
                body: .json(
                    .init(
                        changes: [
                            .init(
                                fieldId: fieldID.uuidString.lowercased(),
                                value: value.map {
                                    .MetadataValue($0.apiValue)
                                } ?? .case2(null))
                        ],
                        revision: revision.map { .case1($0.uuidString.lowercased()) } ?? .case2(null)
                    ))
            ).ok.body.json
            return try Self.customMetadata(snapshot, documentID: documentID)
        }
    }

    private func customField(_ field: Components.Schemas.MetadataField) throws -> CustomFieldDefinition {
        guard try Self.uuid(field.vaultId) == vaultID,
            let kind = CustomFieldKind(rawValue: field.kind.rawValue)
        else { throw FolioError.unreadableResponse }
        return try CustomFieldDefinition(
            id: Self.uuid(field.id),
            draft: .init(name: field.name, kind: kind, required: field.required, unique: field.unique),
            isRetired: field.deletedAt != nil
        )
    }

    private func customOption(
        _ option: Components.Schemas.MetadataOption, fieldID: UUID
    ) throws -> CustomFieldOption {
        guard try Self.uuid(option.vaultId) == vaultID, try Self.uuid(option.fieldId) == fieldID,
            let position = UInt32(exactly: option.position)
        else { throw FolioError.unreadableResponse }
        return try CustomFieldOption(
            id: Self.uuid(option.id), label: option.label, position: position, isRetired: option.deletedAt != nil
        )
    }

    private static func customMetadata(
        _ metadata: Components.Schemas.DocumentMetadata, documentID: UUID
    ) throws -> CustomMetadataSnapshot {
        guard try uuid(metadata.documentId) == documentID else { throw FolioError.unreadableResponse }
        let values = try metadata.values.map { assignment in
            let value: CustomFieldValue?
            switch assignment.value {
            case let .MetadataValue(raw)?: value = try CustomFieldValue(raw)
            case let .case2(raw)? where raw.value == nil: value = nil
            case nil: value = nil
            default: throw FolioError.unreadableResponse
            }
            let link: CustomFieldLink?
            switch assignment.linkTarget {
            case let .MetadataLinkTarget(target)?: link = .init(title: target.title, isTrashed: target.trashed)
            case let .case2(raw)? where raw.value == nil: link = nil
            case nil: link = nil
            default: throw FolioError.unreadableResponse
            }
            return try CustomFieldAssignment(fieldID: uuid(assignment.fieldId), value: value, link: link)
        }
        return try CustomMetadataSnapshot(revision: metadata.revision.map(uuid), values: values)
    }
}

private extension CustomFieldValue {
    init(_ value: Components.Schemas.MetadataValue) throws {
        switch value {
        case let .case1(row): self = .text(row.value)
        case let .case2(row): self = .longText(row.value)
        case let .case3(row): self = .boolean(row.value)
        case let .case4(row): self = .integer(row.value)
        case let .case5(row): self = .decimal(row.value)
        case let .case6(row): self = .date(row.value)
        case let .case7(row): self = .money(amount: row.value.amount, currency: row.value.currency)
        case let .case8(row): self = .select(try LiveFolioService.uuid(row.value))
        case let .case9(row): self = .url(row.value)
        case let .case10(row): self = .documentLink(try LiveFolioService.uuid(row.value))
        }
    }

    var apiValue: Components.Schemas.MetadataValue {
        switch self {
        case let .text(value): .case1(.init(kind: .text, value: value))
        case let .longText(value): .case2(.init(kind: .longText, value: value))
        case let .boolean(value): .case3(.init(kind: .boolean, value: value))
        case let .integer(value): .case4(.init(kind: .integer, value: value))
        case let .decimal(value): .case5(.init(kind: .decimal, value: value))
        case let .date(value): .case6(.init(kind: .date, value: value))
        case let .money(amount, currency): .case7(.init(kind: .money, value: .init(amount: amount, currency: currency)))
        case let .select(value): .case8(.init(kind: .select, value: value.uuidString.lowercased()))
        case let .url(value): .case9(.init(kind: .url, value: value))
        case let .documentLink(value): .case10(.init(kind: .documentLink, value: value.uuidString.lowercased()))
        }
    }
}

extension LiveFolioService: CustomMetadataWorkflowService {
    public func customMetadataWorkflowTriggers() async throws -> [CustomMetadataWorkflowTrigger] {
        try await call("workflows.metadata_triggers") {
            _ = try metadataVault
            let triggers = try await client.listWorkflowTriggers().ok.body.json
            return triggers.compactMap { definition in
                guard definition.conditionFields.contains(where: { $0.key == "event.metadata.field_id" }) else {
                    return nil
                }
                return .init(
                    id: definition.eventType, label: definition.label,
                    supportsValues: definition.conditionFields.contains(where: { $0.key == "event.metadata.after" })
                )
            }
        }
    }

    public func createCustomMetadataWorkflow(_ draft: CustomMetadataWorkflowDraft) async throws {
        try await call("workflows.metadata_create") {
            _ = try metadataVault
            guard let trigger = Components.Schemas.WorkflowTrigger(rawValue: draft.trigger) else {
                throw FolioError.unreadableResponse
            }
            let conditions = Components.Schemas.WorkflowMetadataConditions(
                after: draft.after?.apiValue,
                before: draft.before?.apiValue,
                fieldId: draft.fieldID?.uuidString.lowercased(),
                kind: draft.kind.flatMap { Components.Schemas.MetadataKind(rawValue: $0.rawValue) },
                state: draft.state.flatMap { Components.Schemas.MetadataChangeState(rawValue: $0.rawValue) }
            )
            let event = Components.Schemas.WorkflowEventConditions(metadata: conditions)
            let payload = try JSONDecoder().decode(
                WorkflowConditionsPayload.self, from: JSONEncoder().encode(["event": event])
            )
            let action = Components.Schemas.WorkflowAction.case1(.init(_type: .webhook, url: draft.webhook))
            _ = try await client.createWorkflow(
                body: .json(
                    .init(
                        actions: [action], conditions: payload,
                        enabled: draft.enabled, name: draft.name, trigger: trigger
                    ))
            ).created.body.json
        }
    }
}
