//! Vaults: containers for documents, and the roles that govern them

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Id;

/// Identifier of a vault
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultId(pub Id);

impl VaultId {
    pub fn new() -> Self {
        Self(Id::new_v4())
    }
}

impl Default for VaultId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for VaultId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for VaultId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Id::parse_str(s).map(Self)
    }
}

/// Identifier of a user, as issued by the auth store
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub String);

impl UserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for UserId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifier of a group, as issued by the auth store
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub String);

impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A named container for documents and their vocabularies
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Vault {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, read_only))]
    pub id: VaultId,
    /// Not unique: two people may both want a vault called `Personal`
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, read_only))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(read_only))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(read_only))]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, read_only))]
    pub updated_by: Option<UserId>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(read_only))]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// What a member may do in a vault, ordered `Viewer < Contributor < Editor < Owner`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultRole {
    Viewer,
    Contributor,
    Editor,
    Owner,
}

impl VaultRole {
    pub const ALL: [VaultRole; 4] = [Self::Viewer, Self::Contributor, Self::Editor, Self::Owner];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Contributor => "contributor",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }

    /// Whether this role allows `action` for `user`
    pub fn permits(self, action: &VaultAction, user: &UserId) -> bool {
        use VaultAction::*;
        match action {
            Read | ReadMembers | SaveSearch => true,
            CreateDocument => self >= Self::Contributor,
            EditDocument { uploaded_by } | DeleteDocument { uploaded_by } => match self {
                Self::Viewer => false,
                Self::Contributor => uploaded_by.as_ref() == Some(user),
                Self::Editor | Self::Owner => true,
            },
            ManageVocabulary | MoveDocumentOut | MoveDocumentIn => self >= Self::Editor,
            ManageMembers | ManageWorkflows | ReadAudit | RenameVault | DeleteVault => {
                self == Self::Owner
            }
        }
    }
}

impl std::str::FromStr for VaultRole {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or_else(|| format!("unknown vault role '{s}'"))
    }
}

/// Who a grant is for
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Principal {
    User(UserId),
    Group(GroupId),
}

/// One grant of a role on a vault
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMember {
    pub vault_id: VaultId,
    pub principal: Principal,
    pub role: VaultRole,
    pub granted_by: Option<UserId>,
    pub granted_at: DateTime<Utc>,
}

/// Something a caller wants to do in a vault
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultAction {
    Read,
    CreateDocument,
    EditDocument { uploaded_by: Option<UserId> },
    DeleteDocument { uploaded_by: Option<UserId> },
    ManageVocabulary,
    SaveSearch,
    ManageWorkflows,
    MoveDocumentOut,
    MoveDocumentIn,
    ReadMembers,
    ManageMembers,
    ReadAudit,
    RenameVault,
    DeleteVault,
}

/// Outcome of an access check
///
/// `NotVisible` (no role at all) is kept apart from `Forbidden` (a role that
/// is too low) so the HTTP layer can answer 404 versus 403 from one lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Allowed(VaultRole),
    Forbidden(VaultRole),
    NotVisible,
}

/// An entry of a vault's audit log; holds IDs and role names, never content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Id,
    pub at: DateTime<Utc>,
    pub actor: Option<UserId>,
    pub action: String,
    pub vault_id: Option<VaultId>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
}

/// An audit entry about to be recorded
#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    pub actor: Option<UserId>,
    pub action: String,
    pub vault_id: Option<VaultId>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(own: &UserId, other: &UserId) -> Vec<(VaultAction, [bool; 4])> {
        // Columns: viewer, contributor, editor, owner
        let mine = Some(own.clone());
        let theirs = Some(other.clone());
        vec![
            (VaultAction::Read, [true, true, true, true]),
            (VaultAction::ReadMembers, [true, true, true, true]),
            (VaultAction::SaveSearch, [true, true, true, true]),
            (VaultAction::CreateDocument, [false, true, true, true]),
            (
                VaultAction::EditDocument {
                    uploaded_by: mine.clone(),
                },
                [false, true, true, true],
            ),
            (
                VaultAction::EditDocument {
                    uploaded_by: theirs.clone(),
                },
                [false, false, true, true],
            ),
            (
                VaultAction::EditDocument { uploaded_by: None },
                [false, false, true, true],
            ),
            (
                VaultAction::DeleteDocument { uploaded_by: mine },
                [false, true, true, true],
            ),
            (
                VaultAction::DeleteDocument {
                    uploaded_by: theirs,
                },
                [false, false, true, true],
            ),
            (
                VaultAction::DeleteDocument { uploaded_by: None },
                [false, false, true, true],
            ),
            (VaultAction::ManageVocabulary, [false, false, true, true]),
            (VaultAction::MoveDocumentOut, [false, false, true, true]),
            (VaultAction::MoveDocumentIn, [false, false, true, true]),
            (VaultAction::ManageWorkflows, [false, false, false, true]),
            (VaultAction::ManageMembers, [false, false, false, true]),
            (VaultAction::ReadAudit, [false, false, false, true]),
            (VaultAction::RenameVault, [false, false, false, true]),
            (VaultAction::DeleteVault, [false, false, false, true]),
        ]
    }

    #[test]
    fn role_matrix() {
        let me = UserId::from("alice");
        let other = UserId::from("bob");
        for (action, expected) in actions(&me, &other) {
            for (role, want) in VaultRole::ALL.into_iter().zip(expected) {
                assert_eq!(
                    role.permits(&action, &me),
                    want,
                    "{} on {action:?}",
                    role.as_str()
                );
            }
        }
    }

    #[test]
    fn roles_are_ordered() {
        assert!(VaultRole::Viewer < VaultRole::Contributor);
        assert!(VaultRole::Contributor < VaultRole::Editor);
        assert!(VaultRole::Editor < VaultRole::Owner);
    }

    #[test]
    fn roles_round_trip_through_text() {
        for role in VaultRole::ALL {
            assert_eq!(role.as_str().parse::<VaultRole>(), Ok(role));
        }
        assert!("admin".parse::<VaultRole>().is_err());
    }
}
