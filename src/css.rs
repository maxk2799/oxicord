//! CSS import inliner.
//!
//! Discord's Content-Security-Policy blocks stylesheets, and most images, from
//! third-party hosts, and a system webview can't rewrite response headers. So
//! themes are resolved here instead: every `@import` is downloaded (recursively,
//! in parallel) and spliced in place, relative `url()`s are made absolute, and
//! images on hosts the policy would block are embedded as `data:` URIs.
//!
//! Results are cached on disk so a theme applies instantly on the next launch
//! and is revalidated in the background.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine;
use url::{Host, Url};

const MAX_DEPTH: usize = 6;
const MAX_FETCHES: usize = 96;
const MAX_CSS_BYTES: u64 = 4 << 20;
const MAX_IMAGE_BYTES: u64 = 3 << 20;

pub const SHEET_OPEN: &str = "/*oxicord:sheet*/";
pub const SHEET_CLOSE: &str = "/*oxicord:end*/";

#[derive(Clone)]
struct Entry {
    bytes: Arc<Vec<u8>>,
    /// Fetched from the network during this session.
    fresh: bool,
}

pub struct Resolver {
    agent: ureq::Agent,
    dir: PathBuf,
    mem: Mutex<HashMap<String, Entry>>,
}

struct Ctx<'a> {
    r: &'a Resolver,
    /// Revalidate against the network instead of trusting the disk cache.
    network: bool,
    fetches: AtomicUsize,
    /// Set when a disk-cached (possibly outdated) copy was used.
    stale: AtomicBool,
}

impl Resolver {
    pub fn new(agent: ureq::Agent, dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Self { agent, dir, mem: Mutex::new(HashMap::new()) }
    }

    /// Inline every import in `css`. Returns the result and whether any part
    /// of it came from the disk cache (and so is worth revalidating).
    pub fn inline(&self, css: &str, network: bool) -> (String, bool) {
        let ctx = Ctx {
            r: self,
            network,
            fetches: AtomicUsize::new(0),
            stale: AtomicBool::new(false),
        };
        let mut out = String::with_capacity(css.len());
        let mut rest = css;
        // The page marks embedded local themes so each one is parsed as a
        // stylesheet of its own (imports are only valid at the top of one).
        while let Some(open) = rest.find(SHEET_OPEN) {
            out.push_str(&expand(&ctx, &rest[..open], None, 0));
            let inner = &rest[open + SHEET_OPEN.len()..];
            let (sheet, tail) = match inner.find(SHEET_CLOSE) {
                Some(close) => (&inner[..close], &inner[close + SHEET_CLOSE.len()..]),
                None => (inner, ""),
            };
            out.push_str(&expand(&ctx, sheet, None, 0));
            out.push('\n');
            rest = tail;
        }
        out.push_str(&expand(&ctx, rest, None, 0));
        (out, ctx.stale.load(Ordering::Relaxed))
    }

    fn disk_path(&self, url: &str) -> PathBuf {
        // FNV-1a: stable across builds, unlike the std hasher.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in url.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.dir.join(format!("{h:016x}"))
    }

    fn remember(&self, url: &str, bytes: Arc<Vec<u8>>, fresh: bool) {
        if let Ok(mut mem) = self.mem.lock() {
            mem.insert(url.to_owned(), Entry { bytes, fresh });
        }
    }

    fn download(&self, url: &str, limit: u64) -> Option<Vec<u8>> {
        let mut resp = self.agent.get(url).call().ok()?;
        resp.body_mut().with_config().limit(limit).read_to_vec().ok()
    }
}

fn fetch(ctx: &Ctx, url: &Url, limit: u64) -> Option<Arc<Vec<u8>>> {
    if !fetchable(url) {
        return None;
    }
    let key = url.as_str();
    let cached = ctx.r.mem.lock().ok().and_then(|m| m.get(key).cloned());
    if let Some(entry) = &cached {
        if entry.fresh || !ctx.network {
            if !entry.fresh {
                ctx.stale.store(true, Ordering::Relaxed);
            }
            return Some(entry.bytes.clone());
        }
    }
    let path = ctx.r.disk_path(key);
    if !ctx.network && cached.is_none() {
        if let Ok(bytes) = fs::read(&path) {
            let bytes = Arc::new(bytes);
            ctx.r.remember(key, bytes.clone(), false);
            ctx.stale.store(true, Ordering::Relaxed);
            return Some(bytes);
        }
    }
    if ctx.fetches.fetch_add(1, Ordering::Relaxed) >= MAX_FETCHES {
        return cached.map(|e| e.bytes);
    }
    match ctx.r.download(key, limit) {
        Some(bytes) => {
            let _ = fs::write(&path, &bytes);
            let bytes = Arc::new(bytes);
            ctx.r.remember(key, bytes.clone(), true);
            Some(bytes)
        }
        // Offline or failing host: an old copy beats no theme.
        None => cached
            .map(|e| e.bytes)
            .or_else(|| fs::read(&path).ok().map(Arc::new)),
    }
}

/// Only public https hosts. The page can ask for arbitrary URLs, so this must
/// not become a way to read local or private-network services.
fn fetchable(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d.contains('.')
                && d != "localhost"
                && !d.ends_with(".localhost")
                && !d.ends_with(".local")
                && !d.ends_with(".internal")
                && !d.ends_with(".lan")
        }
        _ => false,
    }
}

/// Hosts Discord's `img-src` policy already allows; no need to embed these.
fn image_host_allowed(host: &str) -> bool {
    const EXACT: &[&str] = &[
        "discord.com",
        "i.scdn.co",
        "i.ytimg.com",
        "i.imgur.com",
        "media.tenor.co",
        "media.tenor.com",
        "c.tenor.com",
        "static.klipy.com",
        "static-cdn.jtvnw.net",
        "pbs.twimg.com",
    ];
    const SUFFIX: &[&str] = &[
        ".discordapp.net",
        ".discordapp.com",
        ".discord.com",
        ".youtube.com",
        ".giphy.com",
    ];
    EXACT.contains(&host) || SUFFIX.iter().any(|s| host.ends_with(s))
}

fn image_mime(url: &Url) -> Option<&'static str> {
    let path = url.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Stylesheet expansion
// ---------------------------------------------------------------------------

enum Item<'a> {
    Keep(&'a str),
    Import { target: &'a str, cond: &'a str },
}

fn expand(ctx: &Ctx, css: &str, base: Option<&Url>, depth: usize) -> String {
    let (items, body_start) = prologue(css);
    let body = &css[body_start..];

    let targets: Vec<Option<Url>> = items
        .iter()
        .map(|item| match item {
            Item::Import { target, .. } => resolve(target, base),
            Item::Keep(_) => None,
        })
        .collect();

    // Download sibling imports concurrently; each recurses into its own.
    let loaded: Vec<Option<String>> = thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|t| t.as_ref().map(|url| s.spawn(move || load_sheet(ctx, url, depth + 1))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.and_then(|h| h.join().ok()))
            .collect()
    });

    let mut out = String::with_capacity(css.len());
    for (item, sheet) in items.iter().zip(loaded) {
        match item {
            Item::Keep(text) => out.push_str(text),
            Item::Import { target, cond } => match sheet {
                Some(sheet) => {
                    out.push_str(&wrap(&sheet, cond));
                    out.push('\n');
                }
                None => {
                    out.push_str("/* oxicord: could not load ");
                    out.push_str(&target.replace("*/", ""));
                    out.push_str(" */\n");
                }
            },
        }
    }
    out.push_str(&rewrite_urls(ctx, body, base));
    out
}

fn load_sheet(ctx: &Ctx, url: &Url, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "/* oxicord: import nesting too deep */".to_owned();
    }
    match fetch(ctx, url, MAX_CSS_BYTES) {
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            expand(ctx, text, Some(url), depth)
        }
        None => format!("/* oxicord: could not load {} */", url.as_str().replace("*/", "")),
    }
}

fn resolve(target: &str, base: Option<&Url>) -> Option<Url> {
    let target = target.trim();
    match base {
        Some(base) => base.join(target).ok(),
        None => Url::parse(target).ok(),
    }
}

/// Apply an import's trailing conditions (`layer(..)`, `supports(..)`, media
/// query) to the inlined content.
fn wrap(content: &str, cond: &str) -> String {
    let mut cond = cond.trim();
    if cond.is_empty() {
        return content.to_owned();
    }
    let mut layer: Option<&str> = None;
    let mut supports: Option<&str> = None;
    if starts_with_ci(cond, "layer(") {
        if let Some(end) = matching_paren(cond, 5) {
            layer = Some(cond[6..end].trim());
            cond = cond[end + 1..].trim_start();
        }
    } else if starts_with_ci(cond, "layer")
        && cond[5..].chars().next().is_none_or(|c| c.is_whitespace())
    {
        layer = Some("");
        cond = cond[5..].trim_start();
    }
    if starts_with_ci(cond, "supports(") {
        if let Some(end) = matching_paren(cond, 8) {
            supports = Some(cond[9..end].trim());
            cond = cond[end + 1..].trim_start();
        }
    }
    let mut out = content.to_owned();
    if !cond.is_empty() {
        out = format!("@media {cond} {{\n{out}\n}}");
    }
    if let Some(s) = supports {
        // `supports(display: grid)` takes a bare declaration; a condition is
        // already parenthesised.
        out = if s.starts_with('(') || starts_with_ci(s, "not ") {
            format!("@supports {s} {{\n{out}\n}}")
        } else {
            format!("@supports ({s}) {{\n{out}\n}}")
        };
    }
    if let Some(name) = layer {
        out = format!("@layer {name} {{\n{out}\n}}");
    }
    out
}

// ---------------------------------------------------------------------------
// Scanning helpers. Every index lands on an ASCII byte, so slicing is safe.
// ---------------------------------------------------------------------------

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b >= 0x80
}

/// Index just past the string that opens at `i`.
fn skip_string(b: &[u8], i: usize) -> usize {
    let quote = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

/// Index just past the comment that opens at `i`.
fn skip_comment(b: &[u8], i: usize) -> usize {
    let mut j = i + 2;
    while j + 1 < b.len() {
        if b[j] == b'*' && b[j + 1] == b'/' {
            return j + 2;
        }
        j += 1;
    }
    b.len()
}

/// Index of the `)` matching the `(` at `open`.
fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_string(b, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the first top-level `;` or `{` at or after `i`, or the end.
fn statement_end(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_string(b, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = skip_comment(b, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b';' | b'{' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// Split the leading run of `@charset` / `@layer x;` / `@import` statements
/// off a stylesheet. Returns the items and where the rest of the sheet begins.
fn prologue(css: &str) -> (Vec<Item<'_>>, usize) {
    let b = css.as_bytes();
    let mut items = Vec::new();
    let mut i = 0;
    loop {
        let start = i;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return (items, start);
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i = skip_comment(b, i);
            items.push(Item::Keep(&css[start..i]));
            continue;
        }
        let rest = &css[i..];
        let at_rule = |name: &str| {
            starts_with_ci(rest, name) && !b.get(i + name.len()).copied().is_some_and(is_ident)
        };
        if at_rule("@charset") {
            let end = statement_end(b, i);
            if b.get(end) != Some(&b';') {
                return (items, start);
            }
            i = end + 1;
        } else if at_rule("@layer") {
            let end = statement_end(b, i);
            if b.get(end) != Some(&b';') {
                return (items, start);
            }
            i = end + 1;
            items.push(Item::Keep(&css[start..i]));
        } else if at_rule("@import") {
            let end = statement_end(b, i);
            if b.get(end) == Some(&b'{') {
                return (items, start);
            }
            let params = &css[i + "@import".len()..end];
            i = (end + 1).min(b.len());
            match import_params(params) {
                Some((target, cond)) => items.push(Item::Import { target, cond }),
                None => items.push(Item::Keep(&css[start..i])),
            }
        } else {
            return (items, start);
        }
    }
}

/// Split `url("x") screen` / `"x" layer(a)` into the target and conditions.
fn import_params(params: &str) -> Option<(&str, &str)> {
    let p = params.trim();
    let b = p.as_bytes();
    if starts_with_ci(p, "url(") {
        let close = matching_paren(p, 3)?;
        let inner = p[4..close].trim();
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(inner);
        Some((inner, &p[close + 1..]))
    } else if matches!(b.first(), Some(b'"' | b'\'')) {
        let end = skip_string(b, 0);
        if end < 2 {
            return None;
        }
        Some((&p[1..end - 1], &p[end..]))
    } else {
        None
    }
}

enum Piece<'a> {
    Text(&'a str),
    Url(&'a str),
}

fn tokenize_urls(body: &str) -> Vec<Piece<'_>> {
    let b = body.as_bytes();
    let mut pieces = Vec::new();
    let (mut i, mut last) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => i = skip_comment(b, i),
            b'"' | b'\'' => i = skip_string(b, i),
            b'u' | b'U'
                if starts_with_ci(&body[i..], "url(") && (i == 0 || !is_ident(b[i - 1])) =>
            {
                let mut j = i + 4;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let (from, to);
                if matches!(b.get(j), Some(b'"' | b'\'')) {
                    let end = skip_string(b, j);
                    from = j + 1;
                    to = end.saturating_sub(1).max(from);
                    j = end;
                } else {
                    from = j;
                    while j < b.len() && b[j] != b')' && !b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    to = j;
                }
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) == Some(&b')') && to <= b.len() {
                    pieces.push(Piece::Text(&body[last..i]));
                    pieces.push(Piece::Url(&body[from..to]));
                    i = j + 1;
                    last = i;
                } else {
                    i += 4;
                }
            }
            _ => i += 1,
        }
    }
    pieces.push(Piece::Text(&body[last..]));
    pieces
}

/// Make `url()` references absolute and embed images the page policy would
/// otherwise block.
fn rewrite_urls(ctx: &Ctx, body: &str, base: Option<&Url>) -> String {
    if !body.as_bytes().windows(4).any(|w| w.eq_ignore_ascii_case(b"url(")) {
        return body.to_owned();
    }
    let pieces = tokenize_urls(body);

    let absolute = |raw: &str| -> Option<Url> {
        let raw = raw.trim();
        if raw.is_empty()
            || raw.starts_with('#')
            || starts_with_ci(raw, "data:")
            || starts_with_ci(raw, "blob:")
            || starts_with_ci(raw, "about:")
        {
            return None;
        }
        resolve(raw, base)
    };
    let needs_embed = |url: &Url| {
        fetchable(url)
            && image_mime(url).is_some()
            && !url.host_str().is_some_and(image_host_allowed)
    };

    let resolved: Vec<Option<Url>> = pieces
        .iter()
        .map(|p| match p {
            Piece::Url(raw) => absolute(raw),
            Piece::Text(_) => None,
        })
        .collect();

    let mut wanted: Vec<&Url> = resolved.iter().flatten().filter(|u| needs_embed(u)).collect();
    wanted.sort();
    wanted.dedup();
    let embedded: HashMap<&Url, String> = thread::scope(|s| {
        let handles: Vec<_> = wanted
            .iter()
            .map(|url| (*url, s.spawn(move || fetch(ctx, url, MAX_IMAGE_BYTES))))
            .collect();
        handles
            .into_iter()
            .filter_map(|(url, h)| {
                let bytes = h.join().ok()??;
                let mime = image_mime(url)?;
                let data = base64::engine::general_purpose::STANDARD.encode(&*bytes);
                Some((url, format!("data:{mime};base64,{data}")))
            })
            .collect()
    });

    let mut out = String::with_capacity(body.len());
    for (piece, url) in pieces.iter().zip(&resolved) {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Url(raw) => {
                let value = match url {
                    Some(url) => embedded.get(url).map_or(url.as_str(), String::as_str),
                    None => raw,
                };
                out.push_str("url(\"");
                for c in value.chars() {
                    match c {
                        '"' => out.push_str("%22"),
                        '\n' | '\r' => {}
                        c => out.push(c),
                    }
                }
                out.push_str("\")");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline() -> Resolver {
        let dir = std::env::temp_dir().join(format!("oxicord-test-{}", std::process::id()));
        Resolver::new(ureq::Agent::new_with_defaults(), dir)
    }

    #[test]
    fn prologue_splits_imports() {
        let css = "/* hi */\n@charset \"utf-8\";\n@import url(\"https://a.example/x.css\") screen;\n@import 'b.css';\n.a{color:red}";
        let (items, start) = prologue(css);
        let imports: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Import { target, cond } => Some((*target, cond.trim())),
                _ => None,
            })
            .collect();
        assert_eq!(imports, vec![("https://a.example/x.css", "screen"), ("b.css", "")]);
        assert_eq!(css[start..].trim(), ".a{color:red}");
    }

    #[test]
    fn body_without_imports_is_untouched() {
        let css = ".a { background: red } /* @import url(x) */";
        assert_eq!(offline().inline(css, false).0, css);
    }

    #[test]
    fn wraps_conditions() {
        assert_eq!(wrap("a{}", " screen and (min-width: 1px)"), "@media screen and (min-width: 1px) {\na{}\n}");
        assert_eq!(wrap("a{}", "layer(base)"), "@layer base {\na{}\n}");
        assert_eq!(wrap("a{}", "supports(display: grid)"), "@supports (display: grid) {\na{}\n}");
    }

    #[test]
    fn rebases_relative_urls() {
        let r = offline();
        let ctx = Ctx { r: &r, network: false, fetches: AtomicUsize::new(MAX_FETCHES), stale: AtomicBool::new(false) };
        let base = Url::parse("https://cdn.discordapp.com/themes/main.css").unwrap();
        let out = rewrite_urls(&ctx, "a{background:url(img/bg.png)} b{content:\"url(x)\"} c{mask:url('#m')}", Some(&base));
        assert_eq!(
            out,
            "a{background:url(\"https://cdn.discordapp.com/themes/img/bg.png\")} b{content:\"url(x)\"} c{mask:url(\"#m\")}"
        );
    }

    #[test]
    fn refuses_private_hosts() {
        for u in ["http://example.com/a.css", "https://localhost/a.css", "https://127.0.0.1/a.css", "https://printer.local/a.css", "https://[::1]/a.css"] {
            assert!(!fetchable(&Url::parse(u).unwrap()), "{u}");
        }
        assert!(fetchable(&Url::parse("https://raw.githubusercontent.com/a/b.css").unwrap()));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Needs the network: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn inlines_a_real_theme() {
        let dir = std::env::temp_dir().join(format!("oxicord-live-{}", std::process::id()));
        let r = Resolver::new(ureq::Agent::new_with_defaults(), dir.clone());
        let src = "@import url(\"https://catppuccin.github.io/discord/dist/catppuccin-mocha.theme.css\");\n.mine{color:red}";
        let (css, stale) = r.inline(src, false);
        assert!(!stale);
        assert!(css.len() > 5_000, "got {} bytes: {}", css.len(), &css[..css.len().min(300)]);
        assert!(!css.contains("@import"));
        assert!(css.ends_with(".mine{color:red}"));
        // A second resolver sees only the disk cache and reports it as stale.
        let r2 = Resolver::new(ureq::Agent::new_with_defaults(), dir.clone());
        let (again, stale) = r2.inline(src, false);
        assert!(stale);
        assert_eq!(again, css);
        let _ = std::fs::remove_dir_all(dir);
    }
}
