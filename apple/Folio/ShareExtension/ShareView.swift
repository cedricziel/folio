import FolioKit
import SwiftUI

struct ShareView: View {
    @State var model: ShareViewModel
    let onFinish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space4) {
            Text(summary)
                .font(FolioTheme.Typography.heading)
                .foregroundStyle(Color.ink)

            if model.files.count == 1 {
                TextField("Title", text: $model.title)
                    .textFieldStyle(.roundedBorder)
                    .font(FolioTheme.Typography.body)
            }

            VaultDestinationPicker(destination: model.destination)
                .disabled(model.state == .uploading)

            if !model.tags.isEmpty {
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
                    Text("TAGS")
                        .font(FolioTheme.Typography.label)
                        .foregroundStyle(Color.inkMuted)
                    FlowLayout(spacing: FolioTheme.Spacing.space2) {
                        ForEach(model.tags) { tag in
                            Toggle(
                                tag.name,
                                isOn: Binding(
                                    get: { model.selectedTagIDs.contains(tag.id) },
                                    set: { isOn in
                                        if isOn {
                                            model.selectedTagIDs.insert(tag.id)
                                        } else {
                                            model.selectedTagIDs.remove(tag.id)
                                        }
                                    }
                                )
                            )
                            .toggleStyle(.button)
                            .font(FolioTheme.Typography.bodySm)
                        }
                    }
                }
            }

            if !model.retained.isEmpty {
                RetainedSharesView(model: model)
            }

            if case let .failed(message) = model.state {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(FolioTheme.Typography.bodySm)
                    .foregroundStyle(Color.danger)
            }

            HStack {
                Button("Cancel", role: .cancel, action: onFinish)
                Spacer()
                Button(model.state == .uploading ? "Uploading…" : "Upload") {
                    Task {
                        await model.upload()
                        if model.state == .done || model.state == .queued {
                            onFinish()
                        }
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.state == .uploading || !model.destination.canUpload)
            }
        }
        .padding(FolioTheme.Spacing.space4)
        .frame(minWidth: 320)
        .background(Color.paper)
        .tint(.archive)
        .task(id: model.destination.selectedVaultID) { await model.loadTags() }
    }

    private var summary: String {
        model.files.count == 1 ? "File this document" : "File \(model.files.count) documents"
    }
}

/// Earlier background shares the server has not accepted yet, each with its outcome and a retry.
private struct RetainedSharesView: View {
    let model: ShareViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: FolioTheme.Spacing.space2) {
            Text("EARLIER SHARES")
                .font(FolioTheme.Typography.label)
                .foregroundStyle(Color.inkMuted)
            ForEach(model.retained) { record in
                VStack(alignment: .leading, spacing: FolioTheme.Spacing.space1) {
                    Text(record.displayName)
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(Color.ink)
                    Text(status(of: record))
                        .font(FolioTheme.Typography.bodySm)
                        .foregroundStyle(isFailure(record) ? Color.danger : Color.inkMuted)
                    HStack {
                        if record.canRetry {
                            Button("Retry") { Task { await model.retry(record.id) } }
                                .disabled(model.retrying.contains(record.id))
                        }
                        if record.status != .pending {
                            Button(record.status == .uploaded ? "Dismiss" : "Discard", role: .destructive) {
                                model.discard(record.id)
                            }
                        }
                    }
                    .font(FolioTheme.Typography.bodySm)
                }
                .accessibilityElement(children: .contain)
            }
            if let error = model.retryError {
                Text(error)
                    .font(FolioTheme.Typography.bodySm)
                    .foregroundStyle(Color.danger)
            }
        }
    }

    private func status(of record: ShareUploadRecord) -> String {
        switch record.status {
        case .pending: "Uploading in the background…"
        case .uploaded: "Uploaded"
        case let .failed(failure): failure.message
        }
    }

    private func isFailure(_ record: ShareUploadRecord) -> Bool {
        if case .failed = record.status { return true }
        return false
    }
}

/// Wraps tag toggles onto as many lines as needed.
private struct FlowLayout: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) -> CGSize {
        arrange(proposal: proposal, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) {
        let result = arrange(proposal: proposal, subviews: subviews)
        for (subview, origin) in zip(subviews, result.origins) {
            subview.place(at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y), proposal: .unspecified)
        }
    }

    private func arrange(proposal: ProposedViewSize, subviews: Subviews) -> (size: CGSize, origins: [CGPoint]) {
        let maxWidth = proposal.width ?? 320
        var origins: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > maxWidth {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        return (CGSize(width: maxWidth, height: y + rowHeight), origins)
    }
}
