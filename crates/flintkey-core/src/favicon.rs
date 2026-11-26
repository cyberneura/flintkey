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

/// Whether `url` may be requested while looking for the icon of the site whose root is `site`: an http(s) URL on
/// the site's own host, or on a host that is not this machine or the local network. A page may not use its icon
/// links, or redirects, to make the app send requests to such addresses on its behalf. Every redirect hop is checked
/// too. (A host name that resolves to a local address is not caught here.)
pub fn may_request(url: &str, site: &str) -> bool {
    checked(url, site).is_some()
}

/// A request that passed [`may_request`], in the form to send: the URL as the check understood it, and, for a host
/// other than the site's, the host and port whose resolved addresses must be checked with [`is_public`] (and the
/// connection pinned to them) before sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub url: String,
    pub resolve: Option<(String, u16)>,
}

/// Parses `url` once and checks it against `site`; the caller sends exactly [`Checked::url`]. URLs with credentials
/// are refused.
pub fn checked(url: &str, site: &str) -> Option<Checked> {
    let (url, site) = (Url::parse(url).ok()?, Url::parse(site).ok()?);
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !may_follow(&url, &site) {
        return None;
    }
    let resolve = match url.host() {
        Some(Host::Domain(domain)) if !same_host(&url, &site) => {
            Some((domain.to_string(), url.port_or_known_default()?))
        }
        _ => None,
    };
    Some(Checked {
        url: url.to_string(),
        resolve,
    })
}

/// Whether `ip` is an address outside this machine and the local network.
pub fn is_public(ip: IpAddr) -> bool {
    public(ip)
}

fn may_follow(candidate: &Url, page: &Url) -> bool {
    if same_host(candidate, page) {
        return true;
    }
    match candidate.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            !(domain == "localhost"
                || domain.ends_with(".localhost")
                || domain.ends_with(".local")
                || domain.ends_with(".internal")
                || domain.ends_with(".home.arpa")
                || !domain.contains('.'))
        }
        Some(Host::Ipv4(ip)) => public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => public(IpAddr::V6(ip)),
        None => false,
    }
}

fn same_host(a: &Url, b: &Url) -> bool {
    let name = |url: &Url| {
        url.host_str()
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
    };
    name(a).is_some() && name(a) == name(b)
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || first == 0
                // Shared address space (carrier-grade NAT), 100.64.0.0/10.
                || (first == 100 && (64..128).contains(&second))
                // Benchmarking, 198.18.0.0/15.
                || (first == 198 && (18..20).contains(&second))
                || first >= 224)
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let first = ip.segments()[0];
            // Unique local (fc00::/7), link-local (fe80::/10), multicast (ff00::/8).
            !(ip.is_loopback()
                || ip.is_unspecified()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
                || first & 0xff00 == 0xff00)
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
            <link rel="icon" href="http://localhost./x.png">
            <link rel="icon" href="http://[::ffff:127.0.0.1]/x.png">
            <link rel="icon" href="http://100.100.1.1/x.png">
            <link rel="icon" href="http://router/x.png">
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
    fn redirect_hops_are_checked_against_the_site() {
        // Act / Assert
        assert!(may_request("https://cdn.example.net/i.png", "https://example.com/"));
        assert!(may_request("http://EXAMPLE.com./x", "https://example.com/"));
        assert!(!may_request(
            "http://169.254.169.254/latest/meta-data",
            "https://example.com/"
        ));
        assert!(!may_request("file:///etc/hosts", "https://example.com/"));
        assert!(may_request("http://10.0.0.5/own.png", "http://10.0.0.5/"));
    }

    #[test]
    fn a_checked_request_is_sent_as_parsed_and_other_hosts_are_resolved_first() {
        // Act
        let other = checked("https://CDN.example.net:8443/a b.png", "https://example.com/").unwrap();
        let own = checked("https://example.com/i.png", "https://example.com/").unwrap();

        // Assert
        assert_eq!(other.url, "https://cdn.example.net:8443/a%20b.png");
        assert_eq!(other.resolve, Some(("cdn.example.net".into(), 8443)));
        assert_eq!(own.resolve, None);
        assert_eq!(
            checked("https://user:pw@cdn.example.net/a.png", "https://example.com/"),
            None
        );
        assert!(!is_public("10.1.2.3".parse().unwrap()));
        assert!(is_public("93.184.216.34".parse().unwrap()));
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
