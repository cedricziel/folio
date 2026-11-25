import SwiftUI

struct WorkflowScheduleFields: View {
    let triggerID: String
    @Binding var cadence: WorkflowCadenceDraft

    var body: some View {
        switch WorkflowCadenceDraft.Trigger(rawValue: triggerID) {
        case .clock:
            Section("Schedule") {
                Picker("Repeat", selection: $cadence.frequency) {
                    ForEach(WorkflowCadenceDraft.Frequency.allCases) { frequency in
                        Text(frequency.rawValue).tag(frequency)
                    }
                }
                .pickerStyle(.segmented)
                utcField("Start in UTC", example: "2030-01-03T09:00:00Z", text: $cadence.start)
                if cadence.frequency == .recurring {
                    TextField("Repeat every (seconds)", text: $cadence.interval)
                }
                validation
            }
        case .documentDate:
            Section {
                TextField("Offset in days", text: $cadence.days)
                utcField("Time in UTC", example: "09:00:00", text: $cadence.time)
                validation
            } header: {
                Text("Document date schedule")
            } footer: {
                Text("Use a negative offset for days before the document date, or zero for the same date.")
            }
        case nil:
            EmptyView()
        }
    }

    @ViewBuilder private var validation: some View {
        if let message = cadence.validationMessage(for: triggerID) {
            Text(message).foregroundStyle(Color.danger)
        }
    }

    private func utcField(_ title: String, example: String, text: Binding<String>) -> some View {
        TextField(title, text: text, prompt: Text(example))
            .accessibilityLabel(title)
            .autocorrectionDisabled()
            #if os(iOS)
                .textInputAutocapitalization(.never)
            #endif
    }
}
