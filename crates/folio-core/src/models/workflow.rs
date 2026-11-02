//! Workflow model: a trigger, optional conditions, and ordered actions

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Document, DocumentSource, Id, SourceConditions, VaultId};
use crate::error::{Error, Result};

/// Events a workflow can listen for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowTrigger {
    /// A document was added
    #[serde(rename = "document.uploaded")]
    DocumentUploaded,
    /// Background processing (thumbnails, OCR, embedding) finished
    #[serde(rename = "document.processed")]
    DocumentProcessed,
    /// A document's metadata changed
    #[serde(rename = "document.updated")]
    DocumentUpdated,
    /// A document was deleted
    #[serde(rename = "document.deleted")]
    DocumentDeleted,
    /// A file or email attachment was found by an ingestion source and is about
    /// to be imported; rules here can discard it
    #[serde(rename = "ingest.received")]
    IngestReceived,
}

impl WorkflowTrigger {
    pub const ALL: [WorkflowTrigger; 5] = [
        Self::DocumentUploaded,
        Self::DocumentProcessed,
        Self::DocumentUpdated,
        Self::DocumentDeleted,
        Self::IngestReceived,
    ];

    /// The wire name, as used in payloads and the database
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DocumentUploaded => "document.uploaded",
            Self::DocumentProcessed => "document.processed",
            Self::DocumentUpdated => "document.updated",
            Self::DocumentDeleted => "document.deleted",
            Self::IngestReceived => "ingest.received",
        }
    }

    /// How a person would say it: "When a document has been read"
    pub fn label(&self) -> &'static str {
        match self {
            Self::DocumentUploaded => "When a document is added",
            Self::DocumentProcessed => "When a document has been read",
            Self::DocumentUpdated => "When a document's details change",
            Self::DocumentDeleted => "When a document is deleted",
            Self::IngestReceived => "When a file arrives, before import",
        }
    }

    /// Parse the wire name
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == value)
    }

    /// Whether a workflow should fire at most once per document for this trigger
    ///
    /// Processing can be re-run (repair, reprocess), but lifecycle milestones
    /// like "uploaded" and "processed" must not notify the same document twice.
    pub fn once_per_document(&self) -> bool {
        matches!(self, Self::DocumentUploaded | Self::DocumentProcessed)
    }
}

/// Conditions a document must meet; every set condition has to match
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowConditions {
    /// MIME type, exact or with a wildcard subtype such as `image/*`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// The document carries this tag
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub tag_id: Option<Id>,

    /// The document has this correspondent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub correspondent_id: Option<Id>,

    /// The document has this document type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_type_id: Option<Id>,

    /// Glob (`*` and `?`) matched case-insensitively against the original filename
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename_glob: Option<String>,

    /// Where the document came from: folder path, email sender, subject, and so on
    #[serde(default, skip_serializing_if = "SourceConditions::is_empty")]
    pub source: SourceConditions,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub all: Vec<WorkflowConditions>,

    /// Omitted means unconstrained; an empty list never matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub any: Option<Vec<WorkflowConditions>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub none: Vec<WorkflowConditions>,
}

impl WorkflowConditions {
    /// Reject invalid source patterns and trees exceeding eight levels or 64 nodes.
    pub fn validate(&self) -> Result<()> {
        let mut pending = vec![(self, 0)];
        let mut count = 0;
        while let Some((conditions, depth)) = pending.pop() {
            count += 1;
            if depth > 8 || count > 64 {
                return Err(Error::Validation(
                    "Workflow conditions allow at most eight nested groups and 64 condition objects"
                        .into(),
                ));
            }
            conditions.source.validate()?;
            for child in conditions
                .all
                .iter()
                .chain(conditions.any.iter().flatten())
                .chain(&conditions.none)
            {
                pending.push((child, depth + 1));
            }
        }
        Ok(())
    }

    /// Whether the document satisfies every set condition
    pub fn matches(&self, document: &Document, tag_ids: &[Id]) -> bool {
        self.matches_metadata(
            document.mime_type.as_deref(),
            document.original_filename.as_deref(),
            &document.source,
            tag_ids,
            document.correspondent_id,
            document.document_type_id,
        )
    }

    /// Whether the event-time metadata satisfies every set condition.
    pub fn matches_snapshot(&self, snapshot: &crate::EventSnapshot) -> bool {
        let tag_ids: Vec<_> = snapshot.tags.iter().map(|tag| tag.id).collect();
        self.matches_metadata(
            snapshot.mime_type.as_deref(),
            snapshot.original_filename.as_deref(),
            &snapshot.source,
            &tag_ids,
            snapshot.correspondent_id,
            snapshot.document_type_id,
        )
    }

    fn matches_metadata(
        &self,
        mime: Option<&str>,
        filename: Option<&str>,
        source: &DocumentSource,
        tag_ids: &[Id],
        correspondent_id: Option<Id>,
        document_type_id: Option<Id>,
    ) -> bool {
        self.matches_file(mime, filename, source)
            && self.tag_id.is_none_or(|id| tag_ids.contains(&id))
            && self
                .correspondent_id
                .is_none_or(|id| correspondent_id == Some(id))
            && self
                .document_type_id
                .is_none_or(|id| document_type_id == Some(id))
            && self.matches_groups(|child| {
                child.matches_metadata(
                    mime,
                    filename,
                    source,
                    tag_ids,
                    correspondent_id,
                    document_type_id,
                )
            })
    }

    fn matches_groups(&self, matches: impl Fn(&Self) -> bool) -> bool {
        self.all.iter().all(&matches)
            && self
                .any
                .as_ref()
                .is_none_or(|group| group.iter().any(&matches))
            && !self.none.iter().any(matches)
    }

    /// Whether a file that is not imported yet satisfies the conditions
    ///
    /// Stored metadata predicates evaluate false because a candidate has no
    /// tag, correspondent, or type; group operators retain their usual semantics.
    pub fn matches_candidate(
        &self,
        mime_type: &str,
        filename: &str,
        source: &DocumentSource,
    ) -> bool {
        self.matches_metadata(Some(mime_type), Some(filename), source, &[], None, None)
    }

    fn matches_file(
        &self,
        mime_type: Option<&str>,
        filename: Option<&str>,
        source: &DocumentSource,
    ) -> bool {
        if let Some(pattern) = &self.mime_type {
            if !mime_type.is_some_and(|mime| glob_match(pattern, mime)) {
                return false;
            }
        }
        if let Some(pattern) = &self.filename_glob {
            if !filename.is_some_and(|name| glob_match(pattern, name)) {
                return false;
            }
        }
        self.source.matches(source)
    }
}

/// Case-insensitive glob where `*` matches any run and `?` any single character
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();

    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = backtrack {
            p = star_p + 1;
            t = star_t + 1;
            backtrack = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// HTTP method used for webhook delivery
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "UPPERCASE")]
pub enum WebhookMethod {
    #[default]
    Post,
}

/// A step a workflow performs, in order
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowAction {
    /// Send the event payload to an HTTP endpoint
    Webhook {
        /// Destination; only `http` and `https` are accepted
        url: String,
        #[serde(default)]
        method: WebhookMethod,
        /// Extra request headers; `X-Folio-*` headers cannot be overridden
        #[serde(default)]
        headers: BTreeMap<String, String>,
        /// Shared secret used to sign deliveries; unsigned when absent
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<String>,
    },
    /// Attach tags to the document
    AddTags {
        #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
        tag_ids: Vec<Id>,
    },
    /// Set the document's correspondent
    SetCorrespondent {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        correspondent_id: Id,
    },
    /// Set the document's type
    SetDocumentType {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        document_type_id: Id,
    },
    /// Discard the file instead of importing it; only valid for `ingest.received`
    SkipImport,
}

impl WorkflowAction {
    /// Whether the action sends the event payload rather than changing the document
    pub fn is_webhook(&self) -> bool {
        matches!(self, Self::Webhook { .. })
    }
}

/// A configured automation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Workflow {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    pub trigger: WorkflowTrigger,
    pub conditions: WorkflowConditions,
    pub actions: Vec<WorkflowAction>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields for creating or replacing a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowInput {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub trigger: WorkflowTrigger,
    #[serde(default)]
    pub conditions: WorkflowConditions,
    pub actions: Vec<WorkflowAction>,
}

fn default_enabled() -> bool {
    true
}

impl WorkflowInput {
    /// Reject workflows that could never run correctly
    ///
    /// Webhook URLs must be `http` or `https`. Private and loopback addresses
    /// are allowed on purpose: Folio is self-hosted and its automation targets
    /// (Home Assistant, n8n) usually live on the same LAN.
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(Error::Validation(msg.to_string()));

        if self.name.trim().is_empty() {
            return invalid("Workflow name must not be empty");
        }
        if self.actions.is_empty() {
            return invalid("A workflow needs at least one action");
        }
        for action in &self.actions {
            match action {
                WorkflowAction::Webhook { url, headers, .. } => {
                    validate_webhook_url(url)?;
                    if headers.keys().any(|k| k.trim().is_empty()) {
                        return invalid("Webhook header names must not be empty");
                    }
                }
                WorkflowAction::AddTags { tag_ids } if tag_ids.is_empty() => {
                    return invalid("add_tags needs at least one tag");
                }
                _ => {}
            }
            if self.trigger == WorkflowTrigger::DocumentDeleted && !action.is_webhook() {
                return invalid("Only webhook actions can run when a document is deleted");
            }
            let skips = matches!(action, WorkflowAction::SkipImport);
            if skips != (self.trigger == WorkflowTrigger::IngestReceived) {
                return invalid(
                    "ingest.received workflows take only skip_import, and skip_import needs that trigger",
                );
            }
        }
        self.conditions.validate()?;
        Ok(())
    }
}

/// The URL with any `user:password@` part masked, safe to show in messages and logs
pub fn redact_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut parsed) => {
            if !parsed.username().is_empty() || parsed.password().is_some() {
                let _ = parsed.set_username("***");
                let _ = parsed.set_password(None);
            }
            parsed.to_string()
        }
        Err(_) => "<invalid url>".to_string(),
    }
}

/// Accept only absolute `http`/`https` URLs with a host
pub fn validate_webhook_url(raw: &str) -> Result<()> {
    let parsed =
        url::Url::parse(raw).map_err(|e| Error::Validation(format!("Invalid webhook URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::Validation(format!(
            "Webhook URL must use http or https, not '{}'",
            parsed.scheme()
        )));
    }
    if parsed.host_str().is_none() {
        return Err(Error::Validation("Webhook URL needs a host".to_string()));
    }
    Ok(())
}

/// Where a run is in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting for a first attempt or a retry
    Pending,
    Succeeded,
    /// Gave up, either on a permanent error or after the last attempt
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// One execution of a workflow for one event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowRun {
    /// Sent as `X-Folio-Run`; deliveries have separate per-action IDs.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub workflow_id: Id,
    pub event: WorkflowTrigger,
    /// Not a foreign key: runs outlive the documents they were about
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_id: Option<Id>,
    pub status: RunStatus,
    pub attempts: u32,
    /// How many actions already finished, so a retry resumes where it stopped
    pub completed_actions: u32,
    pub last_error: Option<String>,
    /// HTTP status of the latest webhook response
    pub response_status: Option<u16>,
    /// The event payload as sent, frozen when the event happened
    #[serde(skip)]
    pub payload: String,
    /// The workflow's actions as they were when the run was created
    ///
    /// Executed instead of the workflow's current list, so editing a workflow
    /// never changes what an already queued run does. May hold webhook secrets,
    /// so it is never serialized.
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub actions: Vec<WorkflowAction>,
    /// Outbox event that caused the run; at most one run per workflow and event
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub source_event_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl WorkflowRun {
    pub fn new(
        vault_id: VaultId,
        workflow_id: Id,
        event: WorkflowTrigger,
        document_id: Option<Id>,
        payload: String,
    ) -> Self {
        Self {
            id: Id::new_v4(),
            vault_id,
            workflow_id,
            event,
            document_id,
            status: RunStatus::Pending,
            attempts: 0,
            completed_actions: 0,
            last_error: None,
            response_status: None,
            payload,
            actions: Vec::new(),
            source_event_id: None,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    /// Freeze the actions to execute
    pub fn with_actions(mut self, actions: Vec<WorkflowAction>) -> Self {
        self.actions = actions;
        self
    }

    /// Record the outbox event that caused this run
    pub fn with_source_event(mut self, event_id: i64) -> Self {
        self.source_event_id = Some(event_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VaultId;

    fn doc(filename: &str, mime: &str) -> Document {
        Document::new(VaultId::new(), "t")
            .with_filename(filename)
            .with_mime_type(mime)
    }

    #[test]
    fn glob_matches_wildcards_case_insensitively() {
        assert!(glob_match("*.pdf", "Invoice.PDF"));
        assert!(glob_match("inv-???.pdf", "inv-042.pdf"));
        assert!(glob_match("image/*", "image/png"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("*.pdf", "invoice.pdf.txt"));
        assert!(!glob_match("inv-???.pdf", "inv-42.pdf"));
        assert!(!glob_match("image/*", "application/pdf"));
    }

    #[test]
    fn empty_conditions_match_everything() {
        assert!(WorkflowConditions::default().matches(&Document::new(VaultId::new(), "t"), &[]));
    }

    #[test]
    fn any_group_requires_one_matching_branch() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "any": [{"mime_type": "image/*"}, {"filename_glob": "invoice-*"}]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("scan.png", "image/png"), &[]));
        assert!(conditions.matches(&doc("invoice-42.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("letter.pdf", "application/pdf"), &[]));
    }

    #[test]
    fn none_group_excludes_matching_documents() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "mime_type": "application/pdf",
            "none": [{"filename_glob": "draft-*"}]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("invoice.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("draft-invoice.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("invoice.png", "image/png"), &[]));
    }

    #[test]
    fn nested_all_group_requires_every_branch() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "all": [{"mime_type": "application/pdf"}, {
                "any": [{"filename_glob": "invoice-*"}, {"filename_glob": "receipt-*"}]
            }]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("receipt-42.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("letter.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("invoice-42.png", "image/png"), &[]));
    }

    #[test]
    fn empty_any_group_never_matches() {
        let conditions: WorkflowConditions =
            serde_json::from_value(serde_json::json!({"any": []})).unwrap();
        assert!(!conditions.matches(&doc("invoice.pdf", "application/pdf"), &[]));
    }

    #[test]
    fn condition_groups_apply_before_import() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "all": [{"mime_type": "application/pdf"}],
            "none": [{"filename_glob": "keep-*"}]
        }))
        .unwrap();
        let source = DocumentSource::default();
        assert!(conditions.matches_candidate("application/pdf", "discard.pdf", &source));
        assert!(!conditions.matches_candidate("application/pdf", "keep-invoice.pdf", &source));
        assert!(!conditions.matches_candidate("image/png", "discard.png", &source));
    }

    #[test]
    fn validation_rejects_excessive_condition_nesting() {
        let mut nested = serde_json::json!({"mime_type": "application/pdf"});
        for _ in 0..9 {
            nested = serde_json::json!({"all": [nested]});
        }
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(nested).unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn validation_rejects_oversized_condition_groups() {
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(serde_json::json!({
            "any": vec![serde_json::json!({"mime_type": "image/*"}); 65]
        }))
        .unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn validation_checks_source_patterns_inside_groups() {
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(serde_json::json!({
            "none": [{"source": {"email_from": {"regex": "["}}}]
        }))
        .unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn groups_match_historical_snapshot_metadata() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "any": [{"filename_glob": "invoice-*"}],
            "none": [{"mime_type": "image/*"}]
        }))
        .unwrap();
        let mut document = doc("invoice-42.pdf", "application/pdf");
        let snapshot = crate::EventSnapshot::from(&crate::events::DeletedDocument {
            document: document.clone(),
            tags: vec![],
        });
        document.original_filename = Some("letter.pdf".into());
        assert!(conditions.matches_snapshot(&snapshot));
        assert!(!conditions.matches(&document, &[]));
    }

    #[test]
    fn legacy_conditions_round_trip_without_group_fields() {
        let legacy = serde_json::json!({"mime_type": "application/pdf"});
        let conditions: WorkflowConditions = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(conditions).unwrap(), legacy);
    }

    #[cfg(feature = "openapi")]
    #[test]
    fn condition_group_schema_uses_recursive_references() {
        use utoipa::OpenApi;

        #[derive(OpenApi)]
        #[openapi(components(schemas(WorkflowConditions)))]
        struct Api;

        let schema = serde_json::to_value(Api::openapi()).unwrap();
        let conditions = &schema["components"]["schemas"]["WorkflowConditions"];
        assert_eq!(
            conditions["properties"]["all"]["items"]["$ref"],
            "#/components/schemas/WorkflowConditions"
        );
        assert_eq!(
            conditions["properties"]["none"]["items"]["$ref"],
            "#/components/schemas/WorkflowConditions"
        );
    }

    #[test]
    fn every_set_condition_must_match() {
        let tag = Id::new_v4();
        let sender = Id::new_v4();
        let document = doc("scan.pdf", "application/pdf").with_correspondent(sender);

        let all = WorkflowConditions {
            mime_type: Some("application/pdf".into()),
            tag_id: Some(tag),
            correspondent_id: Some(sender),
            document_type_id: None,
            filename_glob: Some("*.pdf".into()),
            source: Default::default(),
            ..Default::default()
        };
        assert!(all.matches(&document, &[tag]));
        assert!(!all.matches(&document, &[]), "missing tag");

        let wrong_mime = WorkflowConditions {
            mime_type: Some("image/*".into()),
            ..Default::default()
        };
        assert!(!wrong_mime.matches(&document, &[]));

        let wrong_sender = WorkflowConditions {
            correspondent_id: Some(Id::new_v4()),
            ..Default::default()
        };
        assert!(!wrong_sender.matches(&document, &[]));

        let wrong_type = WorkflowConditions {
            document_type_id: Some(Id::new_v4()),
            ..Default::default()
        };
        assert!(!wrong_type.matches(&document, &[]));
    }

    #[test]
    fn conditions_on_absent_fields_do_not_match() {
        let bare = Document::new(VaultId::new(), "t");
        let by_mime = WorkflowConditions {
            mime_type: Some("*".into()),
            ..Default::default()
        };
        let by_name = WorkflowConditions {
            filename_glob: Some("*".into()),
            ..Default::default()
        };
        assert!(!by_mime.matches(&bare, &[]));
        assert!(!by_name.matches(&bare, &[]));
    }

    fn webhook(url: &str) -> WorkflowAction {
        WorkflowAction::Webhook {
            url: url.to_string(),
            method: WebhookMethod::Post,
            headers: BTreeMap::new(),
            secret: None,
        }
    }

    fn input(trigger: WorkflowTrigger, actions: Vec<WorkflowAction>) -> WorkflowInput {
        WorkflowInput {
            name: "n".into(),
            enabled: true,
            trigger,
            conditions: Default::default(),
            actions,
        }
    }

    #[test]
    fn validation_allows_private_hosts_but_only_http_schemes() {
        for ok in [
            "http://192.168.1.10:8123/api/webhook/x",
            "http://localhost:5678/hook",
            "https://example.com/hook",
        ] {
            let wf = input(WorkflowTrigger::DocumentUploaded, vec![webhook(ok)]);
            assert!(wf.validate().is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://h",
            "not a url",
            "http://",
        ] {
            let wf = input(WorkflowTrigger::DocumentUploaded, vec![webhook(bad)]);
            assert!(wf.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn validation_requires_a_name_and_actions() {
        let mut wf = input(WorkflowTrigger::DocumentUploaded, vec![]);
        assert!(wf.validate().is_err());
        wf.actions = vec![webhook("https://example.com")];
        wf.name = "  ".into();
        assert!(wf.validate().is_err());
    }

    #[test]
    fn deleted_trigger_only_allows_webhooks() {
        let local = WorkflowAction::AddTags {
            tag_ids: vec![Id::new_v4()],
        };
        let wf = input(WorkflowTrigger::DocumentDeleted, vec![local]);
        assert!(wf.validate().is_err());
    }

    #[test]
    fn actions_round_trip_through_json() {
        let json = r#"{"type":"webhook","url":"https://e.com","secret":"s"}"#;
        let action: WorkflowAction = serde_json::from_str(json).unwrap();
        assert_eq!(
            action,
            WorkflowAction::Webhook {
                url: "https://e.com".into(),
                method: WebhookMethod::Post,
                headers: BTreeMap::new(),
                secret: Some("s".into()),
            }
        );
        let back: WorkflowAction =
            serde_json::from_str(&serde_json::to_string(&action).unwrap()).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn redact_url_hides_credentials() {
        assert_eq!(
            redact_url("https://user:pw@example.com/hook?x=1"),
            "https://***@example.com/hook?x=1"
        );
        assert_eq!(redact_url("http://lan:8123/h"), "http://lan:8123/h");
        assert_eq!(redact_url("not a url"), "<invalid url>");
        let err = validate_webhook_url("ftp://user:pw@example.com/x").unwrap_err();
        assert!(!err.to_string().contains("pw"));
    }

    #[test]
    fn trigger_wire_names_round_trip() {
        for t in WorkflowTrigger::ALL {
            assert_eq!(WorkflowTrigger::parse(t.as_str()), Some(t));
            assert_eq!(
                serde_json::to_string(&t).unwrap(),
                format!("\"{}\"", t.as_str())
            );
        }
    }
}
