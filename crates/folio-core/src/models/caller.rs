//! Who is asking, as far as instance-wide permissions go

use super::UserId;

/// The account behind a request or an MCP session
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// `None` when authentication is disabled and nobody signed in
    pub user_id: Option<UserId>,
    pub is_admin: bool,
    /// The first account; always an admin too
    pub is_owner: bool,
}

impl Caller {
    /// An instance admin: the owner or any admin
    pub fn is_instance_admin(&self) -> bool {
        self.is_admin || self.is_owner
    }

    /// Someone without instance-wide rights, such as an unknown account
    pub fn unprivileged(user_id: Option<UserId>) -> Self {
        Self {
            user_id,
            is_admin: false,
            is_owner: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_and_admins_are_instance_admins() {
        let base = Caller::unprivileged(Some(UserId::from("ada")));
        assert!(!base.is_instance_admin());
        assert!(Caller {
            is_admin: true,
            ..base.clone()
        }
        .is_instance_admin());
        assert!(Caller {
            is_owner: true,
            ..base
        }
        .is_instance_admin());
    }
}
