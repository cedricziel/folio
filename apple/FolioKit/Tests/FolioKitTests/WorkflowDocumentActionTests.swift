import FolioAPI
import Foundation
import Testing

struct WorkflowDocumentActionTests {
    @Test func generatedClientKeepsTheOrderedDocumentActions() throws {
        let json = """
            {"name":"File old invoices","trigger":"document.uploaded","actions":[
                {"type":"rename_document","title":"Invoice"},
                {"type":"remove_tags","tag_ids":["11111111-2222-4333-8444-555555555555"]},
                {"type":"clear_correspondent"},
                {"type":"clear_document_type"},
                {"type":"set_document_date","date":"2026-11-20"},
                {"type":"clear_document_date"},
                {"type":"set_reviewed","reviewed":true},
                {"type":"trash_document"}
            ]}
            """
        let request = Data(json.utf8)
        let workflow = try JSONDecoder().decode(Components.Schemas.WorkflowInput.self, from: request)
        let result = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(workflow)) as? NSDictionary)
        let expected = try #require(JSONSerialization.jsonObject(with: request) as? NSDictionary)
        #expect(result["actions"] as? NSArray == expected["actions"] as? NSArray)
    }
}
