//! Where the back link on a document page leads.
//!
//! The target arrives as a query parameter, so it is untrusted: only a handful of
//! list pages of the current vault are accepted, anything else falls back to the Inbox.

use crate::vault::{url, CURRENT_VAULT};
use folio_core::models::VaultId;
use uuid::Uuid;

pub(crate) const INBOX: &str = "Back to Inbox";

pub(crate) struct BackLink {
    pub href: String,
    /// The same page without the vault prefix, as it travels in `return_to`
    pub target: String,
    pub label: &'static str,
}

impl BackLink {
    /// `href` with this list as its return target; the Inbox is what a document falls back to anyway
    pub fn carried_by(&self, href: String) -> String {
        if self.target == "/" {
            return href;
        }
        let target: String = url::form_urlencoded::byte_serialize(self.target.as_bytes()).collect();
        format!("{href}?return_to={target}")
    }
}

pub(crate) fn inbox() -> BackLink {
    BackLink {
        href: url("/"),
        target: "/".to_owned(),
        label: INBOX,
    }
}

/// The list named by a submitted `return_to`, or the Inbox
pub(crate) fn resolve_or_inbox(target: Option<&str>) -> BackLink {
    resolve(target).unwrap_or_else(inbox)
}

/// `return_to` from a form body that may be empty
pub(crate) fn from_form(body: &[u8]) -> Option<String> {
    url::form_urlencoded::parse(body)
        .find(|(key, _)| key == "return_to")
        .map(|(_, value)| value.into_owned())
}

/// The link to the list `target` names, or `None` when it is not a list of this vault.
pub(crate) fn resolve(target: Option<&str>) -> Option<BackLink> {
    let target = target?;
    if target.len() > 2048
        || !target.starts_with('/')
        || target.starts_with("//")
        || target.chars().any(|c| c.is_control() || c == '\\')
    {
        return None;
    }
    let vault = CURRENT_VAULT.try_with(|ctx| ctx.id).ok()?;
    let rest = match target.strip_prefix("/vaults/") {
        Some(scoped) => {
            let (id, rest) = scoped.split_once('/')?;
            if id.parse::<VaultId>().ok()? != vault {
                return None;
            }
            format!("/{rest}")
        }
        None => target.to_owned(),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest.as_str(), ""),
    };
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let label = match path {
        "/" if params
            .iter()
            .any(|(key, value)| FILTERS.contains(&key.as_str()) && !value.is_empty()) =>
        {
            "Back to filtered documents"
        }
        "/" if params
            .iter()
            .any(|(key, value)| key == "view" && value == "all") =>
        {
            "Back to All documents"
        }
        "/" => INBOX,
        "/trash" => "Back to Trash",
        "/saved-views" => "Back to saved views",
        _ => match path.strip_prefix("/saved-views/") {
            Some(id) if id.parse::<Uuid>().is_ok() => "Back to saved view",
            _ => return None,
        },
    };
    Some(BackLink {
        href: url(&rest),
        target: rest,
        label,
    })
}

const FILTERS: [&str; 6] = [
    "tag_id",
    "reviewed",
    "document_date_from",
    "document_date_to",
    "correspondent_id",
    "document_type_id",
];
