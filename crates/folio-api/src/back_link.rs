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
    pub label: &'static str,
}

pub(crate) fn inbox() -> BackLink {
    BackLink {
        href: url("/"),
        label: INBOX,
    }
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
    let params: Vec<&str> = query
        .split('&')
        .filter_map(|p| p.split('=').next())
        .collect();
    let label = match path {
        "/" if params.iter().any(|p| FILTERS.contains(p)) => "Back to filtered documents",
        "/" if query.split('&').any(|p| p == "view=all") => "Back to All documents",
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
