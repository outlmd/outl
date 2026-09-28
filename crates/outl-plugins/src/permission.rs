//! Permission model.
//!
//! A plugin declares the permissions it needs; the user approves them on
//! install; the approved set is frozen in the lockfile. Every host call is
//! gated against the approved set (see [`PermissionSet::check`]).
//!
//! Network access is scoped to a domain. A bare `network:*` is rejected at
//! parse time — wildcards are only allowed *inside* a domain
//! (`network:*.openai.com`).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A capability the plugin is allowed to use against the host.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// Read page/block content.
    ReadPage,
    /// Create or edit page/block content.
    WritePage,
    /// Observe ops as they are applied.
    ReadOpLog,
    /// Submit ops (mutations) to the log.
    SubmitOp,
    /// Per-plugin local key/value storage.
    StorageLocal,
    /// Read the plugin's own secrets from the OS keychain (`ctx.secrets`).
    /// Namespaced per plugin id, so a plugin can never read another's secrets.
    Secrets,
    /// Network access scoped to a single domain (wildcard only as a
    /// leading `*.` label, never the whole host).
    Network(NetworkDomain),
}

/// A network domain a plugin may talk to, e.g. `api.openai.com` or
/// `*.openai.com`. Never a bare `*`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkDomain(String);

impl NetworkDomain {
    /// Parse a domain, rejecting the catch-all `*`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let d = raw.trim();
        if d.is_empty() {
            return Err("empty network domain".into());
        }
        if d == "*" {
            return Err("network:* is not allowed; scope to a domain".into());
        }
        // A `*` is only valid as a leading subdomain label: `*.example.com`.
        if d.contains('*') && !d.starts_with("*.") {
            return Err(format!(
                "invalid network domain `{d}`: wildcard only as leading `*.`"
            ));
        }
        Ok(Self(d.to_string()))
    }

    /// Whether `host` is covered by this domain rule.
    pub fn matches_host(&self, host: &str) -> bool {
        if let Some(suffix) = self.0.strip_prefix("*.") {
            // `*.openai.com` matches `api.openai.com` but not `openai.com`
            // itself nor `evil-openai.com`.
            host.strip_suffix(suffix)
                .is_some_and(|head| head.ends_with('.'))
        } else {
            host == self.0
        }
    }

    /// The raw domain string (`api.openai.com`, `*.openai.com`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The network access one plugin was granted: who, and which hosts.
///
/// The plugin id travels **with** the domains rather than alongside them
/// because every refusal `net.rs` logs has to name the plugin that attempted
/// it — a host log saying "a redirect to attacker.example was blocked" tells
/// the user nothing they can act on, and the whole point of logging a refusal
/// is that the plugin can `catch (e) {}` the copy it is handed. Two parallel
/// parameters is a pair that can be wired up out of sync; one value cannot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetGrant {
    /// The plugin's manifest id, e.g. `acme.sync`.
    pub plugin_id: String,
    /// The hosts it may reach, from its approved `network:<domain>` grants.
    pub domains: Vec<NetworkDomain>,
}

impl Permission {
    /// Parse the wire string form (`read-page`, `network:api.openai.com`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "read-page" => Ok(Self::ReadPage),
            "write-page" => Ok(Self::WritePage),
            "read-op-log" => Ok(Self::ReadOpLog),
            "submit-op" => Ok(Self::SubmitOp),
            "storage:local" => Ok(Self::StorageLocal),
            "secrets" => Ok(Self::Secrets),
            other => match other.strip_prefix("network:") {
                Some(domain) => Ok(Self::Network(NetworkDomain::parse(domain)?)),
                None => Err(format!("unknown permission `{other}`")),
            },
        }
    }

    /// The wire string form.
    pub fn as_wire(&self) -> String {
        match self {
            Self::ReadPage => "read-page".into(),
            Self::WritePage => "write-page".into(),
            Self::ReadOpLog => "read-op-log".into(),
            Self::SubmitOp => "submit-op".into(),
            Self::StorageLocal => "storage:local".into(),
            Self::Secrets => "secrets".into(),
            Self::Network(d) => format!("network:{}", d.as_str()),
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_wire())
    }
}

impl Serialize for Permission {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_wire())
    }
}

impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The set of permissions approved for a plugin. Host calls check against it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(Vec<Permission>);

impl PermissionSet {
    /// Build from an approved list.
    pub fn new(perms: impl IntoIterator<Item = Permission>) -> Self {
        Self(perms.into_iter().collect())
    }

    /// True when `needed` is covered by the approved set. Network permissions
    /// match by domain rule against the requested host.
    pub fn check(&self, needed: &Permission) -> bool {
        match needed {
            Permission::Network(host) => self.0.iter().any(|p| match p {
                Permission::Network(rule) => rule.matches_host(host.as_str()),
                _ => false,
            }),
            other => self.0.contains(other),
        }
    }

    /// True when this set is a superset of `other` — used to detect when an
    /// update asks for permissions beyond what was approved.
    pub fn covers(&self, other: &PermissionSet) -> bool {
        other.0.iter().all(|p| self.0.contains(p))
    }

    /// The approved permissions.
    pub fn as_slice(&self) -> &[Permission] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_permissions() {
        assert_eq!(
            Permission::parse("read-page").unwrap(),
            Permission::ReadPage
        );
        assert_eq!(
            Permission::parse("submit-op").unwrap(),
            Permission::SubmitOp
        );
        assert_eq!(
            Permission::parse("storage:local").unwrap(),
            Permission::StorageLocal
        );
        assert_eq!(Permission::parse("secrets").unwrap(), Permission::Secrets);
    }

    #[test]
    fn unknown_permission_is_rejected() {
        assert!(Permission::parse("delete-everything").is_err());
        // Near-misses must not be silently accepted as `secrets`.
        assert!(Permission::parse("secret").is_err());
        assert!(Permission::parse("secrets:read").is_err());
        assert!(Permission::parse("Secrets").is_err());
    }

    #[test]
    fn secrets_is_gated_and_not_implied() {
        // A plugin without `secrets` is denied; only an exact grant allows it.
        let none = PermissionSet::new([Permission::ReadPage, Permission::StorageLocal]);
        assert!(!none.check(&Permission::Secrets));
        let granted = PermissionSet::new([Permission::Secrets]);
        assert!(granted.check(&Permission::Secrets));
        // `secrets` grants nothing else, and nothing else grants `secrets`.
        assert!(!granted.check(&Permission::StorageLocal));
        assert!(!granted.check(&Permission::ReadPage));
    }

    #[test]
    fn secrets_wire_roundtrips() {
        assert_eq!(Permission::Secrets.as_wire(), "secrets");
        let json = serde_json::to_string(&Permission::Secrets).unwrap();
        assert_eq!(json, r#""secrets""#);
        let back: Permission = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Permission::Secrets);
    }

    #[test]
    fn network_star_is_rejected() {
        assert!(Permission::parse("network:*").is_err());
        assert!(NetworkDomain::parse("*").is_err());
    }

    #[test]
    fn network_mid_wildcard_is_rejected() {
        assert!(NetworkDomain::parse("api.*.com").is_err());
    }

    #[test]
    fn exact_domain_matches_only_itself() {
        let d = NetworkDomain::parse("api.openai.com").unwrap();
        assert!(d.matches_host("api.openai.com"));
        assert!(!d.matches_host("evil.com"));
        assert!(!d.matches_host("api.openai.com.evil.com"));
    }

    #[test]
    fn leading_wildcard_matches_subdomains_only() {
        let d = NetworkDomain::parse("*.openai.com").unwrap();
        assert!(d.matches_host("api.openai.com"));
        assert!(d.matches_host("a.b.openai.com"));
        // The apex is not a subdomain.
        assert!(!d.matches_host("openai.com"));
        // Suffix-collision must not slip through.
        assert!(!d.matches_host("evil-openai.com"));
    }

    #[test]
    fn check_gates_by_approved_set() {
        let set = PermissionSet::new([
            Permission::ReadPage,
            Permission::Network(NetworkDomain::parse("*.openai.com").unwrap()),
        ]);
        assert!(set.check(&Permission::ReadPage));
        assert!(!set.check(&Permission::WritePage));
        assert!(set.check(&Permission::Network(
            NetworkDomain::parse("api.openai.com").unwrap()
        )));
        assert!(!set.check(&Permission::Network(
            NetworkDomain::parse("api.anthropic.com").unwrap()
        )));
    }

    #[test]
    fn covers_detects_permission_growth() {
        let approved = PermissionSet::new([Permission::ReadPage]);
        let wanted = PermissionSet::new([Permission::ReadPage, Permission::SubmitOp]);
        assert!(!approved.covers(&wanted));
        assert!(wanted.covers(&approved));
    }

    #[test]
    fn serde_roundtrip() {
        let perms = vec![
            Permission::ReadPage,
            Permission::Network(NetworkDomain::parse("*.openai.com").unwrap()),
        ];
        let json = serde_json::to_string(&perms).unwrap();
        assert_eq!(json, r#"["read-page","network:*.openai.com"]"#);
        let back: Vec<Permission> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, perms);
    }
}
