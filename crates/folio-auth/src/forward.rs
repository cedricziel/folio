//! Trusted-header ("forward auth") identity extraction.
//!
//! A reverse proxy or SSO gateway (Pangolin, Authelia, Authentik outpost,
//! oauth2-proxy) authenticates the user and passes the identity in headers.
//! They are only honoured when the TCP peer is a configured trusted proxy.

use std::net::IpAddr;

use axum::http::{HeaderMap, HeaderName};
use ipnet::IpNet;

use crate::{external::ExternalIdentity, model::SOURCE_FORWARD};

#[derive(Debug, Clone)]
pub struct ForwardAuth {
    pub user_header: HeaderName,
    pub email_header: HeaderName,
    pub name_header: HeaderName,
    pub groups_header: HeaderName,
    pub admin_group: Option<String>,
    trusted: Vec<IpNet>,
}

/// Settings for [`ForwardAuth`]; header names are validated by [`ForwardAuth::new`].
#[derive(Debug, Clone)]
pub struct ForwardConfig {
    pub user_header: String,
    pub email_header: String,
    pub name_header: String,
    pub groups_header: String,
    /// CIDRs (`10.0.0.0/8`) and bare addresses of the trusted proxies.
    pub trusted_proxies: Vec<String>,
    pub admin_group: Option<String>,
}

impl Default for ForwardConfig {
    fn default() -> Self {
        Self {
            user_header: "Remote-User".into(),
            email_header: "Remote-Email".into(),
            name_header: "Remote-Name".into(),
            groups_header: "Remote-Groups".into(),
            trusted_proxies: Vec::new(),
            admin_group: None,
        }
    }
}

impl ForwardAuth {
    pub fn new(config: ForwardConfig) -> Result<Self, String> {
        let header = |h: &str| {
            HeaderName::from_bytes(h.trim().as_bytes())
                .map_err(|_| format!("invalid header name: {h}"))
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
        Ok(Self {
            user_header: header(&config.user_header)?,
            email_header: header(&config.email_header)?,
            name_header: header(&config.name_header)?,
            groups_header: header(&config.groups_header)?,
            admin_group: config.admin_group.filter(|g| !g.is_empty()),
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

    /// Removes the identity headers so later layers never see unverified values.
    pub fn strip(&self, headers: &mut HeaderMap) {
        for name in [
            &self.user_header,
            &self.email_header,
            &self.name_header,
            &self.groups_header,
        ] {
            headers.remove(name);
        }
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
        let groups = value(&self.groups_header)
            .map(|g| {
                g.split([',', '|'])
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Some(ExternalIdentity {
            source: SOURCE_FORWARD.to_string(),
            external_id: username.to_lowercase(),
            username,
            email: value(&self.email_header),
            display_name: value(&self.name_header),
            groups,
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
    fn parses_identity_and_groups() {
        let mut h = HeaderMap::new();
        h.insert("remote-user", "Alice".parse().unwrap());
        h.insert("remote-groups", "admins, family|kids".parse().unwrap());
        let id = fwd(&["10.0.0.0/8"]).identity(&h).unwrap();
        assert_eq!(id.username, "Alice");
        assert_eq!(id.external_id, "alice");
        assert_eq!(id.groups, ["admins", "family", "kids"]);
        assert!(fwd(&[]).identity(&HeaderMap::new()).is_none());
    }
}
