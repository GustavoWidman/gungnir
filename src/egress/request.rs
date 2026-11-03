//! The request head a client sends before anything is tunnelled.

use super::allowlist::Host;

/// The longest request head the proxy reads before refusing the connection.
pub(super) const MAX_HEAD: usize = 8 * 1024;

/// What a client asked the proxy to reach.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request {
    /// Whether this is a CONNECT, the only method the proxy tunnels.
    pub tunnel: bool,
    pub host: Host,
    pub port: u16,
}

/// The length of the head at the start of `buffer`, through its blank line, once all of it
/// has arrived.
pub(super) fn head_len(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|end| end + 4)
}

/// Parses a request head. `None` when it names no host, so there is nothing to log but the
/// refusal. Requests in absolute form (`GET http://host/`) are parsed so the log can show what
/// the client tried to reach.
pub(super) fn parse(head: &[u8]) -> Option<Request> {
    let line = head.split(|&byte| byte == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let [method, target, version] = line.split(' ').collect::<Vec<_>>()[..] else {
        return None;
    };
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    if method == "CONNECT" {
        let (host, port) = authority(target)?;
        return Some(Request {
            tunnel: true,
            host,
            port: port?,
        });
    }
    let (scheme, rest) = target.split_once("://")?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let (host, port) = authority(rest.split(['/', '?', '#']).next()?)?;
    Some(Request {
        tunnel: false,
        host,
        port: port.unwrap_or(default_port),
    })
}

/// Splits `[userinfo@]host[:port]`, where an IPv6 host is bracketed.
fn authority(authority: &str) -> Option<(Host, Option<u16>)> {
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, rest)| rest);
    let (host, port) = match authority.strip_prefix('[') {
        Some(bracketed) => {
            let (host, rest) = bracketed.split_once(']')?;
            let port = match rest {
                "" => None,
                _ => Some(rest.strip_prefix(':')?),
            };
            (host, port)
        }
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port = match port {
        Some(port) => Some(port.parse().ok()?),
        None => None,
    };
    (!host.is_empty()).then(|| (Host::new(host), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(tunnel: bool, host: &str, port: u16) -> Option<Request> {
        Some(Request {
            tunnel,
            host: Host::new(host),
            port,
        })
    }

    #[test]
    fn finds_the_head_only_once_complete() {
        assert_eq!(head_len(b"CONNECT a:443 HTTP/1.1\r\nHost: a\r\n"), None);
        assert_eq!(
            head_len(b"CONNECT a:443 HTTP/1.1\r\n\r\n\x16\x03"),
            Some(26)
        );
    }

    #[test]
    fn parses_connect_targets() {
        assert_eq!(
            parse(b"CONNECT api.z.ai:443 HTTP/1.1\r\nHost: api.z.ai:443\r\n\r\n"),
            request(true, "api.z.ai", 443)
        );
        assert_eq!(
            parse(b"CONNECT [2001:db8::1]:8443 HTTP/1.0\r\n\r\n"),
            request(true, "2001:db8::1", 8443)
        );
    }

    #[test]
    fn parses_the_host_of_plain_proxy_requests() {
        assert_eq!(
            parse(b"GET http://user@Example.com/path?q HTTP/1.1\r\n\r\n"),
            request(false, "example.com", 80)
        );
        assert_eq!(
            parse(b"POST https://example.com:8443 HTTP/1.1\r\n\r\n"),
            request(false, "example.com", 8443)
        );
    }

    #[test]
    fn rejects_heads_naming_no_reachable_host() {
        for head in [
            &b"CONNECT api.z.ai HTTP/1.1\r\n\r\n"[..],
            b"CONNECT api.z.ai:https HTTP/1.1\r\n\r\n",
            b"CONNECT api.z.ai:70000 HTTP/1.1\r\n\r\n",
            b"CONNECT :443 HTTP/1.1\r\n\r\n",
            b"CONNECT api.z.ai:443 SPDY/3\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: api.z.ai\r\n\r\n",
            b"GET ftp://example.com/ HTTP/1.1\r\n\r\n",
            b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n",
        ] {
            assert_eq!(parse(head), None, "{}", head.escape_ascii());
        }
    }
}
