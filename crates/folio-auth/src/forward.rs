//! Trusted-header ("forward auth") identity extraction.
//!
//! A reverse proxy or SSO gateway (Pangolin, Authelia, Authentik outpost,
//! oauth2-proxy) authenticates the user and passes the identity in headers.
//! They are only honoured when the TCP peer is a configured trusted proxy.

use std::net::IpAddr;

use axum::http::{HeaderMap, HeaderName};
use ipnet::IpNet;

use crate::{crypto::ct_eq, external::ExternalIdentity, model::SOURCE_FORWARD};

#[derive(Debug, Clone)]
pub struct ForwardAuth {
    pub user_header: HeaderName,
    pub email_header: HeaderName,
    pub name_header: HeaderName,
    /// `None` means the groups header is never read.
    pub groups_header: Option<HeaderName>,
    /// `None` means the role header is never read.
    pub role_header: Option<HeaderName>,
    /// When set, accounts are linked by this header and the user header is
    /// only the display username.
    pub user_id_header: Option<HeaderName>,
    admin_group: Option<String>,
    admin_role: Option<String>,
    secret: Option<(HeaderName, String)>,
    trusted: Vec<IpNet>,
}

/// Settings for [`ForwardAuth`]; header names are validated by [`ForwardAuth::new`].
#[derive(Debug, Clone)]
pub struct ForwardConfig {
    pub user_header: String,
    pub email_header: String,
    pub name_header: String,
    /// Empty disables groups: the header is neither read nor trusted.
    pub groups_header: String,
    /// Empty disables roles.
    pub role_header: String,
    /// A stable account id (Pangolin: `Remote-User-Id`). Empty links accounts
    /// by the lower-cased user header, which breaks when a name is reused.
    pub user_id_header: String,
    /// CIDRs (`10.0.0.0/8`) and bare addresses of the trusted proxies.
    pub trusted_proxies: Vec<String>,
    /// Only effective together with `groups_header`.
    pub admin_group: Option<String>,
    /// Only effective together with `role_header`.
    pub admin_role: Option<String>,
    /// When set, the proxy must send this value in `secret_header`.
    pub shared_secret: Option<String>,
    pub secret_header: String,
}

impl Default for ForwardConfig {
    fn default() -> Self {
        Self {
            user_header: "Remote-User".into(),
            email_header: "Remote-Email".into(),
            name_header: "Remote-Name".into(),
            groups_header: String::new(),
            role_header: String::new(),
            user_id_header: String::new(),
            trusted_proxies: Vec::new(),
            admin_group: None,
            admin_role: None,
            shared_secret: None,
            secret_header: "X-Folio-Proxy-Secret".into(),
        }
    }
}

impl ForwardAuth {
    pub fn new(config: ForwardConfig) -> Result<Self, String> {
        let header = |h: &str| {
            HeaderName::from_bytes(h.trim().as_bytes())
                .map_err(|_| format!("invalid header name: {h}"))
        };
        let optional = |h: &str| {
            let h = h.trim();
            (!h.is_empty()).then(|| header(h)).transpose()
        };
        let trusted = config
            .trusted_proxies
            .iter()
            .map(|p| {
                let p = p.trim();
                p.parse::<IpNet>()
                    .or_else(|_| p.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| format!("invalid trusted proxy: {p}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let groups_header = optional(&config.groups_header)?;
        let role_header = optional(&config.role_header)?;
        // A mapping without its header would demote everyone, so it is dropped.
        let admin_group = config
            .admin_group
            .filter(|g| !g.is_empty() && groups_header.is_some());
        let admin_role = config
            .admin_role
            .filter(|r| !r.is_empty() && role_header.is_some());
        let secret = match config.shared_secret.filter(|s| !s.is_empty()) {
            Some(value) => Some((header(&config.secret_header)?, value)),
            None => None,
        };
        Ok(Self {
            user_header: header(&config.user_header)?,
            email_header: header(&config.email_header)?,
            name_header: header(&config.name_header)?,
            groups_header,
            role_header,
            user_id_header: optional(&config.user_id_header)?,
            admin_group,
            admin_role,
            secret,
            trusted,
        })
    }

    pub fn has_trusted_proxies(&self) -> bool {
        !self.trusted.is_empty()
    }

    pub fn is_trusted(&self, peer: IpAddr) -> bool {
        let peer = peer.to_canonical();
        self.trusted.iter().any(|net| net.contains(&peer))
    }

    pub fn has_identity(&self, headers: &HeaderMap) -> bool {
        headers.contains_key(&self.user_header)
    }

    fn identity_headers(&self) -> impl Iterator<Item = &HeaderName> {
        [&self.user_header, &self.email_header, &self.name_header]
            .into_iter()
            .chain(&self.groups_header)
            .chain(&self.role_header)
            .chain(&self.user_id_header)
            .chain(self.secret.as_ref().map(|(name, _)| name))
    }

    /// With a shared secret configured, the proxy must prove itself with it
    /// on top of its address. Compared in constant time.
    pub fn secret_ok(&self, headers: &HeaderMap) -> bool {
        let Some((name, expected)) = &self.secret else {
            return true;
        };
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|given| ct_eq(given, expected))
    }

    /// Removes the identity headers so later layers never see unverified values.
    pub fn strip(&self, headers: &mut HeaderMap) {
        for name in self.identity_headers() {
            headers.remove(name);
        }
    }

    /// True when any identity header appears more than once. Which copy a
    /// proxy and Folio each pick is ambiguous, so such requests are refused.
    pub fn has_repeated_header(&self, headers: &HeaderMap) -> bool {
        self.identity_headers()
            .any(|name| headers.get_all(name).iter().nth(1).is_some())
    }

    /// Reads the identity from headers. Call only for trusted peers.
    pub fn identity(&self, headers: &HeaderMap) -> Option<ExternalIdentity> {
        let value = |name: &HeaderName| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let username = value(&self.user_header)?;
        let external_id = match &self.user_id_header {
            Some(name) => value(name)?,
            None => username.to_lowercase(),
        };
        let groups: Vec<String> = self
            .groups_header
            .as_ref()
            .and_then(&value)
            .map(|g| {
                g.split([',', '|'])
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let role = self.role_header.as_ref().and_then(&value);
        // Admin status is derived from the configured mapping only; with none,
        // `None` leaves it to the first-user rule.
        let admin = match (&self.admin_group, &self.admin_role) {
            (None, None) => None,
            (group, role_name) => Some(
                group.as_ref().is_some_and(|g| groups.contains(g))
                    || role_name.as_ref().is_some_and(|r| role.as_ref() == Some(r)),
            ),
        };
        Some(ExternalIdentity {
            source: SOURCE_FORWARD.to_string(),
            external_id,
            username,
            email: value(&self.email_header),
            display_name: value(&self.name_header),
            groups,
            admin,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fwd(proxies: &[&str]) -> ForwardAuth {
        ForwardAuth::new(ForwardConfig {
            trusted_proxies: proxies.iter().map(|s| s.to_string()).collect(),
            ..ForwardConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn trusts_only_listed_networks() {
        let f = fwd(&["10.0.0.0/8", "192.168.1.5", "fd00::/8"]);
        assert!(f.is_trusted("10.1.2.3".parse().unwrap()));
        assert!(f.is_trusted("192.168.1.5".parse().unwrap()));
        assert!(f.is_trusted("fd00::1".parse().unwrap()));
        assert!(f.is_trusted("::ffff:10.1.2.3".parse().unwrap()));
        assert!(!f.is_trusted("192.168.1.6".parse().unwrap()));
        assert!(!f.is_trusted("203.0.113.9".parse().unwrap()));
        assert!(!fwd(&[]).is_trusted("10.1.2.3".parse().unwrap()));
    }

    #[test]
    fn rejects_bad_config() {
        let bad = |c: ForwardConfig| ForwardAuth::new(c).is_err();
        assert!(bad(ForwardConfig {
            trusted_proxies: vec!["nope".into()],
            ..ForwardConfig::default()
        }));
        assert!(bad(ForwardConfig {
            user_header: "bad header".into(),
            ..ForwardConfig::default()
        }));
    }

    #[test]
    fn detects_repeated_identity_headers() {
        let f = fwd(&[]);
        let mut h = HeaderMap::new();
        h.append("remote-user", "a".parse().unwrap());
        h.append("x-other", "a".parse().unwrap());
        h.append("x-other", "b".parse().unwrap());
        assert!(!f.has_repeated_header(&h));
        h.append("remote-user", "b".parse().unwrap());
        assert!(f.has_repeated_header(&h));
    }

    #[test]
    fn parses_identity_and_groups() {
        let mut h = HeaderMap::new();
        h.insert("remote-user", "Alice".parse().unwrap());
        h.insert("remote-groups", "admins, family|kids".parse().unwrap());
        let configured = ForwardAuth::new(ForwardConfig {
            groups_header: "Remote-Groups".into(),
            ..ForwardConfig::default()
        })
        .unwrap();
        let id = configured.identity(&h).unwrap();
        assert_eq!(id.username, "Alice");
        assert_eq!(id.external_id, "alice");
        assert_eq!(id.groups, ["admins", "family", "kids"]);
        assert!(fwd(&[]).identity(&h).unwrap().groups.is_empty());
        assert!(fwd(&[]).identity(&HeaderMap::new()).is_none());
    }
}
