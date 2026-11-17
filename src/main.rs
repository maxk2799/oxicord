//! Oxicord: a small native shell around Discord's web client with Vencord
//! built in. The system webview does the rendering, so there is no bundled
//! browser engine: the binary is a few megabytes and starts instantly.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod css;
#[cfg(target_os = "macos")]
mod glass;
mod notify;
mod state;
mod vencord;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use url::Url;
use wry::{
    BackgroundThrottlingPolicy, NewWindowResponse, PageLoadEvent, PermissionKind,
    PermissionResponse, WebViewBuilder,
};

use state::State;

const APP_NAME: &str = "Oxicord";
/// Discord's dark background, so the window never flashes white.
#[cfg(not(target_os = "macos"))]
const BACKGROUND: (u8, u8, u8, u8) = (0x1a, 0x1a, 0x1e, 0xff);
const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
const MIN_SIZE: (f64, f64) = (480.0, 360.0);

/// WKWebView's default user agent omits the Safari token, which makes Discord
/// treat it as an unsupported browser for voice and video.
#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                          (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

enum UserEvent {
    /// Answer a request from the page.
    Reply { id: u64, done: bool, value: Option<String> },
    Title(String),
    Menu(String),
    /// The main frame started loading something that isn't Discord.
    Stray(String),
    Ready(String),
    /// Discord's interface has drawn (page-side milliseconds since load began).
    Interactive(u64),
    /// The page's top bar was pressed: move the window.
    Drag,
    /// The page's top bar was double-clicked.
    ZoomWindow,
    /// A report on the page a few seconds after load.
    Probe(String),
}

/// Print a diagnostic line, and append it to the file named by OXICORD_LOG if set (a Windows app has no console).
fn log(line: &str) {
    eprintln!("[oxicord] {line}");
    if let Some(path) = std::env::var_os("OXICORD_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn home_url() -> &'static str {
    match std::env::var("OXICORD_CHANNEL").as_deref() {
        Ok("canary") => "https://canary.discord.com/app",
        Ok("ptb") => "https://ptb.discord.com/app",
        _ => "https://discord.com/app",
    }
}

fn is_discord_host(host: &str) -> bool {
    host == "discord.com" || host.ends_with(".discord.com")
}

fn is_discord_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .is_some_and(|u| u.scheme() == "https" && u.host_str().is_some_and(is_discord_host))
}

/// Hand a link to the system browser. Web links only: never let page content
/// launch arbitrary URL schemes or local files.
fn open_external(url: &str) {
    if let Ok(parsed) = Url::parse(url) {
        if matches!(parsed.scheme(), "http" | "https") {
            let _ = open::that_detached(parsed.as_str());
        }
    }
}

fn dirs_for_app() -> (PathBuf, PathBuf) {
    let config = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("oxicord");
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("oxicord");
    (config, cache)
}

/// `(3) Discord | general` -> `3`, `• Discord | general` -> `•`
fn badge_for(title: &str) -> Option<String> {
    let title = title.trim_start();
    if let Some(rest) = title.strip_prefix('(') {
        let (count, _) = rest.split_once(')')?;
        return (!count.is_empty() && count.len() <= 5).then(|| count.to_owned());
    }
    title.starts_with('•').then(|| "•".to_owned())
}

fn download_target(url: &str, suggested: &std::path::Path) -> Option<PathBuf> {
    let dir = dirs::download_dir()?;
    let name = suggested
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| {
            let parsed = Url::parse(url).ok()?;
            let last = parsed.path_segments()?.next_back()?.to_owned();
            (!last.is_empty()).then_some(last)
        })
        .unwrap_or_else(|| "download".to_owned());
    // Keep it a plain file name inside the downloads folder.
    let name: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
    let name = name.trim_start_matches('.').to_owned();
    let name = if name.is_empty() { "download".to_owned() } else { name };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut path = dir.join(&name);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    Some(path)
}

fn capture(window: &Window, zoom: f64, skin: bool) -> State {
    let size = window.inner_size();
    State {
        position: window.outer_position().ok().map(|p| (p.x, p.y)),
        size: (size.width, size.height),
        maximized: window.is_maximized(),
        zoom,
        skin,
    }
}

/// Only restore a position that is still on a connected display.
fn on_screen(window: &Window, (x, y): (i32, i32)) -> bool {
    window.available_monitors().any(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x - 50
            && y >= p.y - 50
            && x < p.x + s.width as i32 - 100
            && y < p.y + s.height as i32 - 100
    })
}

#[cfg(target_os = "macos")]
fn build_menu(vencord_version: &str, skin: bool) -> muda::Menu {
    use muda::accelerator::{Accelerator, CMD_OR_CTRL, Code, Modifiers};
    use muda::{AboutMetadata, CheckMenuItem, Menu, MenuItem, PredefinedMenuItem as P, Submenu};

    let item = |id: &str, text: &str, mods: Modifiers, key: Code| {
        MenuItem::with_id(id, text, true, Some(Accelerator::new(mods, key)))
    };
    let about = AboutMetadata {
        name: Some(APP_NAME.into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some(format!("Vencord {vencord_version}")),
        ..Default::default()
    };
    let app = Submenu::with_items(
        APP_NAME,
        true,
        &[
            &P::about(None, Some(about)),
            &P::separator(),
            &P::services(None),
            &P::separator(),
            &P::hide(None),
            &P::hide_others(None),
            &P::show_all(None),
            &P::separator(),
            &P::quit(None),
        ],
    );
    // Without an Edit menu, copy/paste shortcuts do nothing in a webview.
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &P::undo(None),
            &P::redo(None),
            &P::separator(),
            &P::cut(None),
            &P::copy(None),
            &P::paste(None),
            &P::paste_and_match_style(None),
            &P::select_all(None),
        ],
    );
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &item("reload", "Reload", CMD_OR_CTRL, Code::KeyR),
            &item("devtools", "Developer Tools", CMD_OR_CTRL | Modifiers::ALT, Code::KeyI),
            &CheckMenuItem::with_id(
                "skin",
                "Glass Skin",
                true,
                skin,
                Some(Accelerator::new(CMD_OR_CTRL | Modifiers::SHIFT, Code::KeyG)),
            ),
            &P::separator(),
            &item("zoom-in", "Zoom In", CMD_OR_CTRL, Code::Equal),
            &item("zoom-out", "Zoom Out", CMD_OR_CTRL, Code::Minus),
            &item("zoom-reset", "Actual Size", CMD_OR_CTRL, Code::Digit0),
            &P::separator(),
            &P::fullscreen(None),
        ],
    );
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &P::minimize(None),
            &P::maximize(None),
            &P::close_window(None),
            &P::separator(),
            &P::bring_all_to_front(None),
        ],
    );
    let menu = Menu::new();
    if let (Ok(app), Ok(edit), Ok(view), Ok(window)) = (app, edit, view, window) {
        let _ = menu.append_items(&[&app, &edit, &view, &window]);
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
    }
    menu
}

fn handle_ipc(
    request: wry::http::Request<String>,
    proxy: &EventLoopProxy<UserEvent>,
    resolver: &Arc<css::Resolver>,
) {
    // Only Discord's own pages may talk to the native side.
    let from_discord = request.uri().scheme_str() == Some("https")
        && request.uri().host().is_some_and(is_discord_host);
    if !from_discord {
        log(&format!("ignored a message from {}", request.uri()));
        return;
    }
    let Ok(msg) = serde_json::from_str::<Value>(request.body()) else { return };
    let text = |k: &str| msg.get(k).and_then(Value::as_str).unwrap_or_default();
    match text("t") {
        "notify" => notify::show(text("title"), text("body")),
        "ready" => {
            let _ = proxy.send_event(UserEvent::Ready(msg.to_string()));
        }
        "drag" => {
            let _ = proxy.send_event(UserEvent::Drag);
        }
        "zoom-window" => {
            let _ = proxy.send_event(UserEvent::ZoomWindow);
        }
        "interactive" => {
            let _ = proxy.send_event(UserEvent::Interactive(msg.get("ms").and_then(Value::as_u64).unwrap_or(0)));
        }
        "probe" => {
            let _ = proxy.send_event(UserEvent::Probe(msg.to_string()));
        }
        "css" => {
            let Some(id) = msg.get("id").and_then(Value::as_u64) else { return };
            let source = text("css").to_owned();
            let (proxy, resolver) = (proxy.clone(), resolver.clone());
            let _ = thread::Builder::new().name("css".into()).spawn(move || {
                // Answer from cache right away, then revalidate if needed.
                let (first, stale) = resolver.inline(&source, false);
                let _ = proxy.send_event(UserEvent::Reply {
                    id,
                    done: !stale,
                    value: Some(first.clone()),
                });
                if stale {
                    let (fresh, _) = resolver.inline(&source, true);
                    let value = (fresh != first).then_some(fresh);
                    let _ = proxy.send_event(UserEvent::Reply { id, done: true, value });
                }
            });
        }
        _ => {}
    }
}

fn main() -> wry::Result<()> {
    let started = Instant::now();
    // OXICORD_SELFTEST=1 prints the page report and exits; any other value is a file to write it to (a Windows
    // app has no console to print on).
    let selftest = std::env::var_os("OXICORD_SELFTEST");
    let (config_dir, cache_dir) = dirs_for_app();

    let saved = State::load(&config_dir);
    let bundle = vencord::load(&cache_dir);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .https_only(true)
        .user_agent(concat!("oxicord/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let resolver = Arc::new(css::Resolver::new(agent.clone(), cache_dir.join("css")));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let mut window_builder = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_min_inner_size(tao::dpi::LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        .with_maximized(saved.maximized);
    #[cfg(target_os = "macos")]
    {
        // A see-through window: the system blur behind it is what the glass
        // skin shows through. The title bar blends into it but still drags.
        use tao::platform::macos::WindowBuilderExtMacOS;
        window_builder = window_builder
            .with_transparent(true)
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            // No separate title strip: the page runs to the top edge and
            // the window buttons sit inside Discord's own top bar.
            .with_fullsize_content_view(true)
            .with_traffic_light_inset(tao::dpi::LogicalPosition::new(14.0, 16.0));
    }
    #[cfg(not(target_os = "macos"))]
    {
        window_builder = window_builder.with_background_color(BACKGROUND);
    }
    #[cfg(windows)]
    {
        // the icon build.rs put in the .exe, for the title bar and taskbar
        use tao::platform::windows::IconExtWindows;
        window_builder = window_builder.with_window_icon(tao::window::Icon::from_resource(1, None).ok());
    }
    window_builder = if saved.size.0 > 0 {
        window_builder.with_inner_size(PhysicalSize::new(saved.size.0, saved.size.1))
    } else {
        window_builder.with_inner_size(tao::dpi::LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1))
    };
    let window = window_builder.build(&event_loop).expect("failed to create window");
    if let Some(pos) = saved.position.filter(|p| on_screen(&window, *p)) {
        window.set_outer_position(PhysicalPosition::new(pos.0, pos.1));
    }

    // Three document-start scripts, main frame only: bridge and shims, then
    // Vencord itself (unmodified, behind a strict-mode guard), then the glue
    // that needs Vencord to exist.
    let preamble = include_str!("js/preamble.js")
        .replace("__OXICORD_VERSION__", env!("CARGO_PKG_VERSION"))
        .replace("__VENCORD_VERSION__", &bundle.version);
    let vencord_js = format!(
        "\"use strict\";if(!window.__oxicord)throw new Error(\"oxicord: not a Discord page\");\n{}",
        bundle.js
    );
    // The glass skin needs the see-through window, which only macOS has here.
    let mut skin = saved.skin && cfg!(target_os = "macos");
    let js_string = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    let postamble = include_str!("js/postamble.js")
        .replace("__VENCORD_CSS__", &js_string(&bundle.css))
        .replace("__OXICORD_SKIN_CSS__", &js_string(include_str!("skin.css")))
        .replace("__OXICORD_SKIN_ON__", if skin { "true" } else { "false" })
        .replace("__OXICORD_INSET__", if cfg!(target_os = "macos") { "true" } else { "false" });

    // Windows and Linux keep the browser profile (the login) where the app says; macOS has its own place for it.
    #[cfg(not(target_os = "macos"))]
    let mut web_context = wry::WebContext::new(Some(
        dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("oxicord").join("webview"),
    ));
    #[cfg(not(target_os = "macos"))]
    let builder = WebViewBuilder::new_with_web_context(&mut web_context);
    #[cfg(target_os = "macos")]
    let builder = WebViewBuilder::new();
    let mut builder = builder
        .with_url(home_url())
        .with_initialization_script_for_main_only(preamble, true)
        .with_initialization_script_for_main_only(vencord_js, true)
        .with_initialization_script_for_main_only(postamble, true)
        .with_devtools(true)
        .with_autoplay(true)
        .with_clipboard(true)
        .with_back_forward_navigation_gestures(false)
        // Keep the gateway connection serviced while the window is hidden.
        .with_background_throttling(BackgroundThrottlingPolicy::Throttle)
        .with_ipc_handler({
            let (proxy, resolver) = (proxy.clone(), resolver.clone());
            move |request| handle_ipc(request, &proxy, &resolver)
        })
        // This fires for iframes too (captchas, embeds), so it can only filter
        // by scheme; stray main-frame loads are caught in the load handler.
        .with_navigation_handler(|url| {
            Url::parse(&url)
                .is_ok_and(|u| matches!(u.scheme(), "https" | "http" | "about" | "blob" | "data"))
        })
        .with_on_page_load_handler({
            let proxy = proxy.clone();
            move |event, url| {
                if std::env::var_os("OXICORD_LOG").is_some() {
                    let stage = if matches!(event, PageLoadEvent::Started) { "started" } else { "finished" };
                    log(&format!("page load {stage}: {url}"));
                }
                if matches!(event, PageLoadEvent::Started)
                    && url.starts_with("http")
                    && !is_discord_url(&url)
                {
                    let _ = proxy.send_event(UserEvent::Stray(url));
                }
            }
        })
        // Links and popups go to the real browser.
        .with_new_window_req_handler(|url, _features| {
            open_external(&url);
            NewWindowResponse::Deny
        })
        .with_document_title_changed_handler({
            let proxy = proxy.clone();
            move |title| {
                let _ = proxy.send_event(UserEvent::Title(title));
            }
        })
        .with_permission_handler(|kind| match kind {
            PermissionKind::Microphone
            | PermissionKind::Camera
            | PermissionKind::DisplayCapture
            | PermissionKind::Notifications
            | PermissionKind::ClipboardRead
            | PermissionKind::Autoplay
            | PermissionKind::PointerLock => PermissionResponse::Allow,
            _ => PermissionResponse::Default,
        })
        .with_download_started_handler(|url, path| match download_target(&url, path) {
            Some(target) => {
                *path = target;
                true
            }
            None => false,
        });
    #[cfg(target_os = "macos")]
    {
        builder = builder.with_user_agent(USER_AGENT).with_transparent(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.with_background_color(BACKGROUND).with_hotkeys_zoom(true);
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let webview = builder.build(&window)?;
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("window has no gtk container"))?
    };

    #[cfg(target_os = "macos")]
    glass::apply(&window);
    let mut zoom = saved.zoom;
    if (zoom - 1.0).abs() > f64::EPSILON {
        let _ = webview.zoom(zoom);
    }

    #[cfg(target_os = "macos")]
    muda::MenuEvent::set_event_handler(Some({
        let proxy = proxy.clone();
        move |event: muda::MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(event.id.0));
        }
    }));
    #[cfg(target_os = "macos")]
    let mut menu = None;

    vencord::spawn_updater(agent, cache_dir, bundle.version.clone());
    log(&format!(
        "window up in {} ms, Vencord {} ({})",
        started.elapsed().as_millis(),
        bundle.version,
        if bundle.cached { "updated copy" } else { "built in" }
    ));
    let vencord_version = bundle.version;
    drop((bundle.js, bundle.css));

    let mut last_saved = saved;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut persist = |window: &Window, zoom: f64, skin: bool| {
            let now = capture(window, zoom, skin);
            if now != last_saved {
                now.save(&config_dir);
                last_saved = now;
            }
        };

        match event {
            Event::NewEvents(StartCause::Init) => {
                #[cfg(target_os = "macos")]
                {
                    menu = Some(build_menu(&vencord_version, skin));
                }
                let _ = &vencord_version;
                notify::init();
            }

            Event::UserEvent(UserEvent::Reply { id, done, value }) => {
                let value = value
                    .and_then(|v| serde_json::to_string(&v).ok())
                    .unwrap_or_else(|| "null".into());
                let _ = webview.evaluate_script(&format!(
                    "window.__oxicord&&window.__oxicord._reply({id},{done},{value})"
                ));
            }

            Event::UserEvent(UserEvent::Title(title)) => {
                #[cfg(target_os = "macos")]
                {
                    use tao::platform::macos::WindowExtMacOS;
                    window.set_badge_label(badge_for(&title));
                }
                let _ = badge_for;
                let shown = if title.is_empty() { APP_NAME } else { &title };
                window.set_title(shown);
            }

            Event::UserEvent(UserEvent::Menu(id)) => {
                let target = match id.as_str() {
                    "reload" => {
                        let _ = webview.reload();
                        None
                    }
                    "devtools" => {
                        webview.open_devtools();
                        None
                    }
                    "skin" => {
                        skin = !skin;
                        let _ = webview.evaluate_script(&format!(
                            "document.documentElement.toggleAttribute('data-ox-glass',{skin})"
                        ));
                        None
                    }
                    "zoom-in" => Some(zoom + 0.1),
                    "zoom-out" => Some(zoom - 0.1),
                    "zoom-reset" => Some(1.0),
                    _ => None,
                };
                if let Some(z) = target {
                    zoom = z.clamp(0.5, 2.5);
                    let _ = webview.zoom(zoom);
                }
            }

            Event::UserEvent(UserEvent::Stray(url)) => {
                // Something navigated the app itself away from Discord: send it
                // to the browser and come back.
                open_external(&url);
                let _ = webview.load_url(home_url());
            }

            Event::UserEvent(UserEvent::Ready(report)) => {
                log(&format!(
                    "page ready {} ms after launch: {report}",
                    started.elapsed().as_millis()
                ));
            }

            Event::UserEvent(UserEvent::Interactive(page_ms)) => {
                log(&format!(
                    "Discord usable {} ms after launch ({page_ms} ms of that in the page)",
                    started.elapsed().as_millis()
                ));
            }

            Event::UserEvent(UserEvent::Drag) => {
                let _ = window.drag_window();
            }

            Event::UserEvent(UserEvent::ZoomWindow) => {
                window.set_maximized(!window.is_maximized());
            }

            Event::UserEvent(UserEvent::Probe(report)) => {
                log(&format!("page check: {report}"));
                if let Some(target) = &selftest {
                    println!("{report}");
                    if target != "1" {
                        let _ = std::fs::write(target, &report);
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                persist(&window, zoom, skin);
                // On macOS closing the window keeps the app (and its
                // notifications) alive, like Discord; Cmd+Q quits.
                if cfg!(target_os = "macos") {
                    window.set_visible(false);
                } else {
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::Reopen { .. } => {
                window.set_visible(true);
                window.set_focus();
            }

            Event::LoopDestroyed => persist(&window, zoom, skin),

            _ => {}
        }
        #[cfg(target_os = "macos")]
        let _ = &menu;
    });
}

#[cfg(test)]
mod tests {
    use super::badge_for;

    #[test]
    fn badge_follows_discord_title() {
        assert_eq!(badge_for("(3) Discord | general").as_deref(), Some("3"));
        assert_eq!(badge_for("• Discord | Friends").as_deref(), Some("•"));
        assert_eq!(badge_for("Discord | Friends"), None);
        assert_eq!(badge_for("Discord"), None);
    }
}
