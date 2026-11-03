const TLS_PORT: u16 = 443;

/// A host name compared the way DNS compares it: ASCII case-insensitive, trailing dot ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Host(String);

impl Host {
    pub fn new(name: &str) -> Self {
        Self(name.trim_end_matches('.').to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_within(&self, domain: &Host) -> bool {
        self.0
            .strip_suffix(&domain.0)
            .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
    }
}

/// The hosts an attempt may reach through the proxy: each listed host and its subdomains, on
/// the TLS port only.
#[derive(Clone, Debug)]
pub struct Allowlist {
    hosts: Vec<Host>,
    port: u16,
}

impl Allowlist {
    pub fn new<S: AsRef<str>>(hosts: impl IntoIterator<Item = S>) -> Self {
        let hosts = hosts
            .into_iter()
            .map(|host| Host::new(host.as_ref()))
            .filter(|host| !host.0.is_empty())
            .collect();
        Self {
            hosts,
            port: TLS_PORT,
        }
    }

    pub(super) fn allows(&self, host: &Host, port: u16) -> bool {
        port == self.port && self.hosts.iter().any(|allowed| host.is_within(allowed))
    }

    /// Tunnels a port other than 443, so tests can reach a listener on loopback.
    #[cfg(test)]
    pub(super) fn on_port(self, port: u16) -> Self {
        Self { port, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allows(allowlist: &[&str], host: &str, port: u16) -> bool {
        Allowlist::new(allowlist).allows(&Host::new(host), port)
    }

    #[test]
    fn allows_listed_hosts_and_their_subdomains_on_443() {
        assert!(allows(&["api.z.ai"], "api.z.ai", 443));
        assert!(allows(&["api.z.ai"], "eu.api.z.ai", 443));
        assert!(allows(
            &["chatgpt.com", "api.openai.com"],
            "api.openai.com",
            443
        ));
    }

    #[test]
    fn compares_names_like_dns() {
        assert!(allows(&["API.Z.ai."], "api.z.AI.", 443));
    }

    #[test]
    fn refuses_lookalikes_parents_and_other_hosts() {
        assert!(!allows(&["api.z.ai"], "evilapi.z.ai", 443));
        assert!(!allows(&["api.z.ai"], "api.z.ai.evil.com", 443));
        assert!(!allows(&["api.z.ai"], "z.ai", 443));
        assert!(!allows(&["api.z.ai"], "example.com", 443));
        assert!(!allows(&[""], "example.com", 443));
    }

    #[test]
    fn refuses_every_port_but_443() {
        assert!(!allows(&["api.z.ai"], "api.z.ai", 80));
        assert!(!allows(&["api.z.ai"], "api.z.ai", 22));
    }
}
