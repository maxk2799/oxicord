# Oxicord

A small, fast Discord client shell written in Rust, with [Vencord](https://github.com/Vendicated/Vencord) built in.

Oxicord renders Discord's web client in the operating system's own webview (WebKit on macOS) instead of shipping a browser engine, so the whole app is about 3 MB and its window is up in under 200 ms. Vencord is compiled into the binary and injected before Discord's code runs.

> Client modifications are against Discord's Terms of Service. Use at your own risk.

## Build

```sh
scripts/bundle-macos.sh --install   # builds and copies Oxicord.app to /Applications
```

`cargo run --release` also works, but voice, video and notifications need the `.app` bundle: macOS only exposes them to apps that declare what they use.

## What it does

- **Vencord built in.** The bundle in `vendor/vencord` is embedded at compile time, so the first launch needs no download. A background check fetches newer builds from Vencord's official release and applies them on the next launch (`OXICORD_NO_UPDATE=1` disables this; `scripts/update-vencord.sh` refreshes the embedded copy).
- **Themes work despite Discord's content security policy.** A webview can't strip response headers, so `src/css.rs` downloads every `@import` itself, inlines it, and embeds images from hosts the policy would block. Results are cached on disk and revalidated in the background. Web fonts from hosts other than Google Fonts remain blocked.
- **QuickCSS editor.** A small in-page editor replaces Vencord's popup editor, which only works inside the browser extension.
- **Native integration.** Notifications, Dock badge for mentions and unreads, a real Edit menu, downloads to `~/Downloads`, links open in your browser, window size, position and zoom are remembered, closing the window keeps the app running.

## Settings

| Variable | Effect |
| --- | --- |
| `OXICORD_CHANNEL=canary` or `ptb` | Use a different Discord release channel |
| `OXICORD_NO_UPDATE=1` | Never check for Vencord updates |
| `OXICORD_SELFTEST=1` | Load Discord, print whether Vencord initialised, and exit |

## Limits

- Built and tested on macOS only. The code has Windows and Linux paths, but they are untested.
- Vencord runs in its web flavour, so plugins that need Vencord's desktop helper process are not available.
- Screen sharing depends on what the system webview supports.
- The app is ad-hoc signed, not notarised.

## License

GPL-3.0-or-later, the same as Vencord, whose bundle is distributed with it (see `vendor/vencord/browser.js.LEGAL.txt`).
