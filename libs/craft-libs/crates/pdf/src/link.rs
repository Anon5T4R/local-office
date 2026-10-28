//! Which link targets from a document may become clickable links in a written PDF.

/// Whether a link target from a (possibly untrusted) document may be opened by whoever opens
/// the file: `http`/`https` URLs with a host, and `mailto` with an address.
///
/// Everything else is refused: `file:`, `javascript:`, `data:`, relative paths, network
/// shares (`\\host\share`, `//host/share`, `smb:`) and other schemes, which an operating system
/// may hand to a protocol handler. Leading and trailing whitespace is ignored; schemes compare
/// case-insensitively.
pub fn safe_url(url: &str) -> bool {
    let u = url.trim();
    let Some((scheme, rest)) = u.split_once(':') else { return false };
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => rest.strip_prefix("//").is_some_and(|r| !host(r).is_empty()),
        "mailto" => has_recipient(rest),
        _ => false,
    }
}

/// The host of an authority-first URL remainder: before the path, query or fragment, after any
/// userinfo, without the port.
fn host(rest: &str) -> &str {
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    match host_port.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or("", |(h, _)| h),
        None => host_port.split(':').next().unwrap_or(""),
    }
}

/// Whether a `mailto:` remainder names an address, before its headers or in a `to` header.
fn has_recipient(rest: &str) -> bool {
    let (to, headers) = rest.split_once('?').unwrap_or((rest, ""));
    !to.trim().is_empty() || headers.split('&').any(|h| h.split_once('=').is_some_and(|(k, v)| k.eq_ignore_ascii_case("to") && !v.trim().is_empty()))
}

