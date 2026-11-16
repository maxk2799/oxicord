//! The Vencord bundle: compiled into the binary so first launch needs no
//! network, and refreshed in the background from Vencord's official release.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const EMBEDDED_JS: &str = include_str!("../vendor/vencord/browser.js");
const EMBEDDED_CSS: &str = include_str!("../vendor/vencord/browser.css");

const RELEASE: &str = "https://github.com/Vendicated/Vencord/releases/download/devbuild";
const MIN_JS_BYTES: usize = 100 << 10;
const MAX_BYTES: u64 = 8 << 20;

pub struct Bundle {
    pub js: Cow<'static, str>,
    pub css: Cow<'static, str>,
    pub version: String,
    pub cached: bool,
}

/// `// Vencord e75ac9a` -> `e75ac9a`
fn version_of(js: &str) -> Option<&str> {
    let line = js.lines().next()?;
    let v = line.strip_prefix("// Vencord ")?.trim();
    (!v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_graphic())).then_some(v)
}

fn valid_js(js: &str) -> bool {
    js.len() >= MIN_JS_BYTES && version_of(js).is_some() && js.contains("var Vencord=")
}

fn dir(cache: &Path) -> PathBuf {
    cache.join("vencord")
}

/// The newest usable bundle: a downloaded update if there is one, otherwise
/// the copy compiled into the binary.
pub fn load(cache: &Path) -> Bundle {
    let embedded = version_of(EMBEDDED_JS).unwrap_or("unknown");
    let dir = dir(cache);
    // An update is only trusted relative to the build that fetched it; a new
    // build ships a newer embedded copy and starts over.
    let base = fs::read_to_string(dir.join("base")).unwrap_or_default();
    if base.trim() == embedded {
        if let (Ok(js), Ok(css)) = (
            fs::read_to_string(dir.join("browser.js")),
            fs::read_to_string(dir.join("browser.css")),
        ) {
            if valid_js(&js) && !css.is_empty() {
                let version = version_of(&js).unwrap_or("unknown").to_owned();
                return Bundle { js: js.into(), css: css.into(), version, cached: true };
            }
        }
    }
    Bundle {
        js: EMBEDDED_JS.into(),
        css: EMBEDDED_CSS.into(),
        version: embedded.to_owned(),
        cached: false,
    }
}

/// Check for a newer Vencord off the main thread. An update is written to the
/// cache and picked up on the next launch.
pub fn spawn_updater(agent: ureq::Agent, cache: PathBuf, current: String) {
    if std::env::var_os("OXICORD_NO_UPDATE").is_some() {
        return;
    }
    let _ = thread::Builder::new().name("vencord-update".into()).spawn(move || {
        // Let Discord have the network first.
        thread::sleep(Duration::from_secs(8));
        match update(&agent, &cache, &current) {
            Ok(Some(v)) => eprintln!("[oxicord] Vencord {v} downloaded; it applies on next launch"),
            Ok(None) => {}
            Err(e) => eprintln!("[oxicord] Vencord update check failed: {e}"),
        }
    });
}

fn get(agent: &ureq::Agent, url: &str, range: Option<&str>) -> Result<String, String> {
    let mut req = agent.get(url);
    if let Some(range) = range {
        req = req.header("Range", range);
    }
    let mut resp = req.call().map_err(|e| e.to_string())?;
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn update(agent: &ureq::Agent, cache: &Path, current: &str) -> Result<Option<String>, String> {
    let js_url = format!("{RELEASE}/browser.js");
    // The version is on the first line, so 128 bytes answer "is it newer?".
    let head = get(agent, &js_url, Some("bytes=0-127"))?;
    let latest = version_of(&head).ok_or("unexpected bundle header")?.to_owned();
    if latest == current {
        return Ok(None);
    }
    // A server that ignored the range already sent the whole file.
    let js = if valid_js(&head) { head } else { get(agent, &js_url, None)? };
    if !valid_js(&js) || version_of(&js) != Some(latest.as_str()) {
        return Err("downloaded bundle failed validation".into());
    }
    let css = get(agent, &format!("{RELEASE}/browser.css"), None)?;
    if css.is_empty() {
        return Err("downloaded stylesheet is empty".into());
    }

    let dir = dir(cache);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let write = |name: &str, data: &str| -> Result<(), String> {
        let tmp = dir.join(format!("{name}.tmp"));
        fs::write(&tmp, data).map_err(|e| e.to_string())?;
        fs::rename(&tmp, dir.join(name)).map_err(|e| e.to_string())
    };
    // `base` goes last: until it matches, `load` ignores the other files.
    let _ = fs::remove_file(dir.join("base"));
    write("browser.css", &css)?;
    write("browser.js", &js)?;
    write("base", version_of(EMBEDDED_JS).unwrap_or("unknown"))?;
    Ok(Some(latest))
}
