//! Fixed alert texts and payloads
//!
//! Alerts only ever say how many documents reached which outcome. Titles,
//! names and contents never go into a payload.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::models::{Id, NotificationKind, VaultId};

pub const CATEGORY_DOCUMENT: &str = "FOLIO_DOCUMENT";
pub const CATEGORY_INBOX: &str = "FOLIO_INBOX";
pub const CATEGORY_TEST: &str = "FOLIO_TEST";

/// What one alert covers: distinct documents per outcome
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlertSummary {
    documents: BTreeMap<NotificationKind, BTreeSet<Id>>,
    vaults: BTreeSet<VaultId>,
    single: Option<(Id, VaultId)>,
}

impl AlertSummary {
    pub fn add(&mut self, kind: NotificationKind, document: Id, vault: VaultId) {
        self.documents.entry(kind).or_default().insert(document);
        self.vaults.insert(vault);
        let distinct: BTreeSet<&Id> = self.documents.values().flatten().collect();
        self.single = (distinct.len() == 1).then_some((document, vault));
    }

    /// Every document and outcome covered
    pub fn items(&self) -> Vec<(Id, NotificationKind)> {
        self.documents
            .iter()
            .flat_map(|(kind, documents)| documents.iter().map(|document| (*document, *kind)))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn count(&self, kind: NotificationKind) -> usize {
        self.documents.get(&kind).map_or(0, BTreeSet::len)
    }

    /// The vault, when every document is in one
    pub fn single_vault(&self) -> Option<VaultId> {
        (self.vaults.len() == 1)
            .then(|| self.vaults.iter().next().copied())
            .flatten()
    }

    /// The document, when the alert covers exactly one
    pub fn single_document(&self) -> Option<Id> {
        self.single.map(|(id, _)| id)
    }

    /// Single documents and failures are worth waking the device for
    pub fn priority(&self) -> u8 {
        if self.single.is_some() || self.count(NotificationKind::Failed) > 0 {
            10
        } else {
            5
        }
    }

    pub fn body(&self, locale: &str) -> String {
        let language = Language::of(locale);
        self.documents
            .iter()
            .enumerate()
            .map(|(index, (kind, documents))| language.phrase(*kind, documents.len(), index == 0))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The APNs payload; `show_titles` lets the device add the title it fetches itself
    pub fn payload(&self, locale: &str, show_titles: bool) -> Value {
        let document = self.single_document();
        let vault = self.single_vault();
        let mut aps = json!({
            "alert": { "title": "Folio", "body": self.body(locale) },
            "category": if document.is_some() { CATEGORY_DOCUMENT } else { CATEGORY_INBOX },
            "sound": "default",
        });
        if let Some(vault) = vault {
            aps["thread-id"] = json!(format!("vault:{vault}"));
        }
        if show_titles && document.is_some() {
            aps["mutable-content"] = json!(1);
        }
        json!({
            "aps": aps,
            "folio": {
                "v": 1,
                "vault": vault,
                "document": document,
                "counts": {
                    "needs_review": self.count(NotificationKind::NeedsReview),
                    "failed": self.count(NotificationKind::Failed),
                    "processed": self.count(NotificationKind::Processed),
                },
            },
        })
    }
}

/// The payload of a test notification
pub fn test_payload(locale: &str) -> Value {
    let body = match Language::of(locale) {
        Language::En => "Notifications from Folio are working",
        Language::De => "Benachrichtigungen von Folio funktionieren",
    };
    json!({
        "aps": { "alert": { "title": "Folio", "body": body }, "category": CATEGORY_TEST, "sound": "default" },
        "folio": { "v": 1, "test": true },
    })
}

/// The payload that asks WidgetKit to reload
pub fn widgets_payload() -> Value {
    json!({ "aps": { "content-changed": true } })
}

#[derive(Clone, Copy)]
enum Language {
    En,
    De,
}

impl Language {
    fn of(locale: &str) -> Self {
        let language = locale.split(['-', '_']).next().unwrap_or_default();
        if language.eq_ignore_ascii_case("de") {
            Self::De
        } else {
            Self::En
        }
    }

    /// A count phrase; later parts of a sentence use the short form
    fn phrase(self, kind: NotificationKind, n: usize, full: bool) -> String {
        use NotificationKind::*;
        let one = n == 1;
        match (self, kind, full) {
            (Self::En, NeedsReview, true) if one => "1 document needs review".into(),
            (Self::En, NeedsReview, true) => format!("{n} documents need review"),
            (Self::En, NeedsReview, false) => format!("{n} to review"),
            (Self::En, Failed, true) if one => "1 document failed processing".into(),
            (Self::En, Failed, true) => format!("{n} documents failed processing"),
            (Self::En, Failed, false) => format!("{n} failed"),
            (Self::En, Processed, true) if one => "1 document is ready".into(),
            (Self::En, Processed, true) => format!("{n} documents are ready"),
            (Self::En, Processed, false) => format!("{n} ready"),
            (Self::De, NeedsReview, true) if one => "1 Dokument muss geprüft werden".into(),
            (Self::De, NeedsReview, true) => format!("{n} Dokumente müssen geprüft werden"),
            (Self::De, NeedsReview, false) => format!("{n} zu prüfen"),
            (Self::De, Failed, true) if one => "1 Dokument konnte nicht verarbeitet werden".into(),
            (Self::De, Failed, true) => format!("{n} Dokumente konnten nicht verarbeitet werden"),
            (Self::De, Failed, false) => format!("{n} fehlgeschlagen"),
            (Self::De, Processed, true) if one => "1 Dokument ist fertig".into(),
            (Self::De, Processed, true) => format!("{n} Dokumente sind fertig"),
            (Self::De, Processed, false) => format!("{n} fertig"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn one_document_opens_it() {
        let (doc, vault) = (Uuid::new_v4(), VaultId::new());
        let mut summary = AlertSummary::default();
        summary.add(NotificationKind::NeedsReview, doc, vault);
        assert_eq!(summary.body("en"), "1 document needs review");
        assert_eq!(summary.priority(), 10);
        let payload = summary.payload("en", false);
        assert_eq!(payload["folio"]["document"], json!(doc));
        assert_eq!(payload["aps"]["category"], CATEGORY_DOCUMENT);
        assert_eq!(payload["aps"]["thread-id"], json!(format!("vault:{vault}")));
        assert!(payload["aps"].get("mutable-content").is_none());
    }

    #[test]
    fn counts_distinct_documents_per_outcome() {
        let vault = VaultId::new();
        let mut summary = AlertSummary::default();
        for _ in 0..3 {
            summary.add(NotificationKind::NeedsReview, Uuid::new_v4(), vault);
        }
        let failed = Uuid::new_v4();
        summary.add(NotificationKind::Failed, failed, vault);
        summary.add(NotificationKind::Failed, failed, vault);
        assert_eq!(summary.body("en-US"), "3 documents need review, 1 failed");
        assert_eq!(
            summary.body("de_DE"),
            "3 Dokumente müssen geprüft werden, 1 fehlgeschlagen"
        );
        assert_eq!(summary.priority(), 10);
        assert_eq!(
            summary.payload("en", true)["folio"]["document"],
            Value::Null
        );
    }

    #[test]
    fn several_vaults_have_no_thread() {
        let mut summary = AlertSummary::default();
        summary.add(
            NotificationKind::NeedsReview,
            Uuid::new_v4(),
            VaultId::new(),
        );
        summary.add(
            NotificationKind::NeedsReview,
            Uuid::new_v4(),
            VaultId::new(),
        );
        let payload = summary.payload("en", false);
        assert!(payload["aps"].get("thread-id").is_none());
        assert_eq!(payload["folio"]["vault"], Value::Null);
        assert_eq!(payload["aps"]["category"], CATEGORY_INBOX);
        assert_eq!(summary.priority(), 5);
    }

    #[test]
    fn the_same_document_twice_is_still_single() {
        let (doc, vault) = (Uuid::new_v4(), VaultId::new());
        let mut summary = AlertSummary::default();
        summary.add(NotificationKind::NeedsReview, doc, vault);
        summary.add(NotificationKind::Failed, doc, vault);
        assert_eq!(summary.single_document(), Some(doc));
    }

    #[test]
    fn titles_are_only_requested_for_one_document() {
        let mut summary = AlertSummary::default();
        summary.add(
            NotificationKind::NeedsReview,
            Uuid::new_v4(),
            VaultId::new(),
        );
        assert_eq!(summary.payload("en", true)["aps"]["mutable-content"], 1);
        summary.add(
            NotificationKind::NeedsReview,
            Uuid::new_v4(),
            VaultId::new(),
        );
        assert!(summary.payload("en", true)["aps"]
            .get("mutable-content")
            .is_none());
    }
}
