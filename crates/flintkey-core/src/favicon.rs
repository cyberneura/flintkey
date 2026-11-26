//! Finding a site's icon from its page: the `<link rel="icon">` family in the HTML, then `/favicon.ico`.

use std::net::IpAddr;
use url::{Host, Url};

/// The page to read a site's icons from: the root of the origin of `url`, without credentials, path or query. The
/// command's own URL is never requested, since opening it may do something (a sign-out link, a form action).
pub fn site_root(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    let port = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    Some(format!("{}://{host}{port}/", url.scheme()))
}

/// Whether an icon URL found in a page may be fetched: one on the page's own host, or one that is not an address
/// on this machine or the local network. A page may not use its icon links to make the app send requests to such
/// addresses on its behalf. (A host name that resolves to one is not caught here.)
fn may_follow(candidate: &Url, page: &Url) -> bool {
    if candidate.host() == page.host() {
        return true;
    }
    match candidate.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.to_ascii_lowercase();
            domain != "localhost" && !domain.ends_with(".localhost") && !domain.ends_with(".local")
        }
        Some(Host::Ipv4(ip)) => public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => public(IpAddr::V6(ip)),
        None => false,
    }
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()),
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            // Unique local (fc00::/7) and link-local (fe80::/10).
            !(ip.is_loopback() || ip.is_unspecified() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80)
        }
    }
}

/// The icon URLs to try for the page at `page_url` whose HTML is `html`, best first. Larger and scalable icons come
/// first, since the launcher shows them at 32 points on high-density screens. `/favicon.ico` of the page's origin is
/// always last.
pub fn candidates(html: &str, page_url: &str) -> Vec<String> {
    let Ok(page) = Url::parse(page_url) else {
        return Vec::new();
    };
    let base = base_href(html)
        .and_then(|href| page.join(&href).ok())
        .unwrap_or_else(|| page.clone());
    let mut found: Vec<(u32, usize, String)> = Vec::new();
    for (index, tag) in tags(html, "link").enumerate() {
        let (Some(rel), Some(href)) = (attribute(tag, "rel"), attribute(tag, "href")) else {
            continue;
        };
        let rel = rel.to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_ascii_whitespace().collect();
        let touch = rels.iter().any(|rel| rel.starts_with("apple-touch-icon"));
        if !touch && !rels.contains(&"icon") {
            continue;
        }
        let Ok(url) = base.join(&href) else { continue };
        if !matches!(url.scheme(), "http" | "https") || !may_follow(&url, &page) {
            continue;
        }
        let svg = url.path().to_ascii_lowercase().ends_with(".svg")
            || attribute(tag, "type").is_some_and(|kind| kind.eq_ignore_ascii_case("image/svg+xml"));
        let score = if svg {
            u32::MAX
        } else {
            match attribute(tag, "sizes").and_then(|sizes| largest(&sizes)) {
                Some(size) => size,
                // Touch icons are 180 px when no size is given; plain icons are usually 16 or 32 px.
                None if touch => 180,
                None => 32,
            }
        };
        found.push((score, index, url.to_string()));
    }
    // Best score first; between equals, the one written first.
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut urls: Vec<String> = Vec::new();
    for (_, _, url) in found {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if let Ok(fallback) = page.join("/favicon.ico") {
        let fallback = fallback.to_string();
        if !urls.contains(&fallback) {
            urls.push(fallback);
        }
    }
    urls
}

/// The largest width in a `sizes` attribute (`"16x16 32x32"`, `"any"`).
fn largest(sizes: &str) -> Option<u32> {
    sizes
        .split_ascii_whitespace()
        .filter_map(|size| {
            if size.eq_ignore_ascii_case("any") {
                return Some(u32::MAX - 1);
            }
            let lowered = size.to_ascii_lowercase();
            let (width, _) = lowered.split_once('x')?;
            width.parse().ok()
        })
        .max()
}

fn base_href(html: &str) -> Option<String> {
    tags(html, "base").find_map(|tag| attribute(tag, "href"))
}

/// The text of each `<name …>` start tag in `html`, between the name and `>`. Only the head is of interest and
/// icons are declared there, so this does not need a full HTML parser.
fn tags<'a>(html: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let lowered = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(offset) = lowered[from..].find(&open) {
        let start = from + offset + open.len();
        from = start;
        // `<link` must not be the start of a longer name such as `<linkset`.
        if lowered[start..].starts_with(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>') {
            let end = lowered[start..].find('>').map_or(html.len(), |end| start + end);
            starts.push((start, end));
        }
    }
    starts.into_iter().map(move |(start, end)| &html[start..end])
}

/// The value of attribute `name` in the inside of a tag, with `&amp;` decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let lowered = tag.to_ascii_lowercase();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && !matches!(bytes[index], b'=' | b'/') {
            index += 1;
        }
        let key = &lowered[key_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let mut value = None;
        if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let (start, end) = match bytes.get(index) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let start = index + 1;
                    let end = tag[start..].find(quote as char).map_or(tag.len(), |end| start + end);
                    index = (end + 1).min(tag.len());
                    (start, end)
                }
                _ => {
                    let start = index;
                    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                        index += 1;
                    }
                    (start, index)
                }
            };
            value = Some(&tag[start..end]);
        }
        if key == name {
            return Some(value.unwrap_or("").replace("&amp;", "&"));
        }
        if key.is_empty() && value.is_none() {
            index += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_scalable_then_larger_icons_and_ends_with_favicon_ico() {
        // Arrange
        let html = r#"<html><head>
            <link rel="stylesheet" href="/site.css">
            <link rel="icon" href="/small.png" sizes="16x16">
            <LINK REL="shortcut icon" HREF="/legacy.ico">
            <link rel=icon href=/big.png sizes="32x32 192x192">
            <link rel="apple-touch-icon" href="touch.png">
            <link rel="icon" type="image/svg+xml" href="https://cdn.example.net/logo">
        </head></html>"#;

        // Act
        let urls = candidates(html, "https://example.com/docs/page.html");

        // Assert
        assert_eq!(
            urls,
            [
                "https://cdn.example.net/logo",
                "https://example.com/big.png",
                "https://example.com/docs/touch.png",
                "https://example.com/legacy.ico",
                "https://example.com/small.png",
                "https://example.com/favicon.ico",
            ]
        );
    }

    #[test]
    fn a_page_without_icons_falls_back_to_favicon_ico_of_its_origin() {
        // Act / Assert
        assert_eq!(
            candidates("<title>x</title>", "http://intranet.local:8080/a/b?c=d"),
            ["http://intranet.local:8080/favicon.ico"]
        );
        assert!(candidates("", "not a url").is_empty());
    }

    #[test]
    fn the_site_root_is_read_never_the_command_url() {
        // Act / Assert
        assert_eq!(
            site_root("https://user:secret@example.com:8443/account/logout?next=/#x").as_deref(),
            Some("https://example.com:8443/")
        );
        assert_eq!(
            site_root("http://intranet.local/a").as_deref(),
            Some("http://intranet.local/")
        );
        assert_eq!(site_root("file:///etc/passwd"), None);
    }

    #[test]
    fn icon_links_to_local_addresses_on_another_host_are_not_followed() {
        // Arrange
        let html = r#"<link rel="icon" href="http://127.0.0.1:8080/admin.png">
            <link rel="icon" href="http://192.168.1.1/x.png">
            <link rel="icon" href="http://[::1]/x.png">
            <link rel="icon" href="http://localhost/x.png">
            <link rel="icon" href="http://printer.local/x.png">
            <link rel="icon" href="https://cdn.example.net/ok.png">"#;

        // Act
        let public_page = candidates(html, "https://example.com/");
        let local_page = candidates(r#"<link rel="icon" href="/own.png">"#, "http://192.168.1.10/");

        // Assert
        assert_eq!(
            public_page,
            ["https://cdn.example.net/ok.png", "https://example.com/favicon.ico"]
        );
        // A local site's own icons are fine.
        assert_eq!(
            local_page,
            ["http://192.168.1.10/own.png", "http://192.168.1.10/favicon.ico"]
        );
    }

    #[test]
    fn base_href_entities_and_other_schemes_are_handled() {
        // Arrange
        let html = r#"<base href="https://static.example.com/assets/">
            <link rel="icon" href="icon.png?v=1&amp;s=2">
            <link rel="icon" href="data:image/png;base64,AAAA">
            <linkset rel="icon" href="/not-a-link.png">"#;

        // Act
        let urls = candidates(html, "https://example.com/");

        // Assert
        assert_eq!(
            urls,
            [
                "https://static.example.com/assets/icon.png?v=1&s=2",
                "https://example.com/favicon.ico"
            ]
        );
    }
}
