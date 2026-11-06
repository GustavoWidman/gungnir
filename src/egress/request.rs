//! The request head a client sends before anything is tunnelled or relayed.

use super::allowlist::Host;

/// The longest request head the proxy reads before refusing the connection.
pub(super) const MAX_HEAD: usize = 8 * 1024;

/// What a client asked the proxy to reach.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub kind: Kind,
    pub host: Host,
    pub port: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A CONNECT, which the proxy tunnels to an allowlisted host.
    Tunnel,
    /// A request in absolute form (`POST http://host/path`), which only the relay serves.
    Forward(Forward),
}

/// A request in absolute form.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Forward {
    pub method: String,
    /// Whether the target's scheme is plain `http`, the only one the relay serves.
    pub plain: bool,
    /// The path and query after the authority: `/` when the target names none.
    pub path: String,
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
/// refusal.
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
            kind: Kind::Tunnel,
            host,
            port: port?,
        });
    }
    let (scheme, rest) = target.split_once("://")?;
    let (plain, default_port) = match scheme.to_ascii_lowercase().as_str() {
        "http" => (true, 80),
        "https" => (false, 443),
        _ => return None,
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, port) = authority(&rest[..end])?;
    let path = rest[end..].split('#').next().unwrap_or_default();
    let path = match path.strip_prefix('?') {
        Some(_) => format!("/{path}"),
        None if path.is_empty() => "/".to_owned(),
        None => path.to_owned(),
    };
    Some(Request {
        kind: Kind::Forward(Forward {
            method: method.to_owned(),
            plain,
            path,
        }),
        host,
        port: port.unwrap_or(default_port),
    })
}

/// The header fields of a request head, names in lowercase and values as sent, without the
/// whitespace around them. `None` when a line is no `name: value` field.
pub(super) fn fields(head: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut fields = Vec::new();
    for line in head.split(|&byte| byte == b'\n').skip(1) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let colon = line.iter().position(|&byte| byte == b':')?;
        let name = std::str::from_utf8(&line[..colon]).ok()?;
        if name.is_empty() || !name.bytes().all(is_token) {
            return None;
        }
        let value = line[colon + 1..].trim_ascii();
        fields.push((name.to_ascii_lowercase(), value.to_owned()));
    }
    Some(fields)
}

/// A byte of an HTTP token, which a field name is.
fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
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

    fn tunnel(host: &str, port: u16) -> Option<Request> {
        Some(Request {
            kind: Kind::Tunnel,
            host: Host::new(host),
            port,
        })
    }

    fn forward(method: &str, plain: bool, host: &str, port: u16, path: &str) -> Option<Request> {
        Some(Request {
            kind: Kind::Forward(Forward {
                method: method.to_owned(),
                plain,
                path: path.to_owned(),
            }),
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
            tunnel("api.z.ai", 443)
        );
        assert_eq!(
            parse(b"CONNECT [2001:db8::1]:8443 HTTP/1.0\r\n\r\n"),
            tunnel("2001:db8::1", 8443)
        );
    }

    #[test]
    fn parses_the_target_of_requests_in_absolute_form() {
        assert_eq!(
            parse(b"GET http://user@Example.com/path?q HTTP/1.1\r\n\r\n"),
            forward("GET", true, "example.com", 80, "/path?q")
        );
        assert_eq!(
            parse(b"POST https://example.com:8443 HTTP/1.1\r\n\r\n"),
            forward("POST", false, "example.com", 8443, "/")
        );
        assert_eq!(
            parse(b"POST http://relay.invalid/responses HTTP/1.1\r\nhost: relay.invalid\r\n\r\n"),
            forward("POST", true, "relay.invalid", 80, "/responses")
        );
        assert_eq!(
            parse(b"GET http://relay.invalid?limit=1#top HTTP/1.1\r\n\r\n"),
            forward("GET", true, "relay.invalid", 80, "/?limit=1")
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

    #[test]
    fn reads_header_fields_with_lowercase_names() {
        let head = b"POST http://relay.invalid/responses HTTP/1.1\r\n\
            Content-Type: application/json\r\n\
            X-Empty:\r\n\
            Content-Length:  12 \r\n\r\n";
        assert_eq!(
            fields(head),
            Some(vec![
                ("content-type".to_owned(), b"application/json".to_vec()),
                ("x-empty".to_owned(), Vec::new()),
                ("content-length".to_owned(), b"12".to_vec()),
            ])
        );
        assert_eq!(fields(b"GET http://a/ HTTP/1.1\r\nno colon\r\n\r\n"), None);
        assert_eq!(
            fields(b"GET http://a/ HTTP/1.1\r\nbad name: x\r\n\r\n"),
            None
        );
        assert_eq!(fields(b"GET http://a/ HTTP/1.1\r\n: x\r\n\r\n"), None);
    }
}
