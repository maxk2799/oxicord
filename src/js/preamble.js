// Oxicord preamble. Runs at document start, before Vencord and before Discord.
// Sets up the bridge to the Rust side and the shims the system webview needs.
(() => {
    "use strict";
    if (window.__oxicord) return;

    const host = location.hostname;
    if (host !== "discord.com" && !host.endsWith(".discord.com")) return;

    const post = msg => window.ipc.postMessage(JSON.stringify(msg));
    const pending = new Map();
    let seq = 0;

    const call = (type, payload, onResult) => {
        const id = ++seq;
        pending.set(id, onResult);
        post({ t: type, id, ...payload });
        return id;
    };

    Object.defineProperty(window, "__oxicord", {
        value: Object.freeze({
            version: "__OXICORD_VERSION__",
            vencord: "__VENCORD_VERSION__",
            t0: performance.now(),
            post,
            // Called from Rust. `done` drops the callback, `value` may be null.
            _reply(id, done, value) {
                const cb = pending.get(id);
                if (!cb) return;
                if (done) pending.delete(id);
                if (value !== null) cb(value);
            },
        }),
    });

    // ---------------------------------------------------------------------
    // Startup speed. Vencord patches Discord's modules with regexes, and a few
    // use a variable-length lookbehind: /(?<=A.+?)B/. WebKit's regex engine
    // runs those in seconds, not milliseconds, on Discord's large modules
    // (two of them cost over 3 s of every launch). For that exact shape, do
    // the same job with two ordinary searches: find A, then B after it.
    // ---------------------------------------------------------------------
    // Only WebKit needs it; Chromium (Windows) runs these regexes quickly as they are.
    if (/Apple/.test(navigator.vendor)) {
        const nativeReplace = RegExp.prototype[Symbol.replace];
        const parsed = new Map();
        const NEWLINE = /[\n\r\u2028\u2029]/;

        // Split "(?<=A.+?)B" into A, B and the minimum gap; null if it isn't that shape.
        const parse = re => {
            const src = re.source;
            if (!src.startsWith("(?<=") || re.global || re.sticky) return null;
            let depth = 0, inClass = false, end = -1;
            for (let i = 0; i < src.length; i++) {
                const c = src[i];
                if (c === "\\") { i++; continue; }
                if (inClass) { if (c === "]") inClass = false; continue; }
                if (c === "[") inClass = true;
                else if (c === "(") depth++;
                else if (c === ")" && --depth === 0) { end = i; break; }
            }
            if (end < 0) return null;
            const inner = src.slice(4, end);
            const lazy = inner.endsWith(".+?") ? 1 : inner.endsWith(".*?") ? 0 : -1;
            // an escaped dot ("\.+?") is not a wildcard
            if (lazy < 0 || inner.slice(0, -3).endsWith("\\")) return null;
            try {
                const flags = re.flags.replace(/[gy]/g, "");
                return {
                    a: new RegExp(inner.slice(0, -3), flags),
                    b: new RegExp(src.slice(end + 1), flags + "g"),
                    gap: lazy,
                    dotAll: re.dotAll,
                };
            } catch {
                return null;
            }
        };

        const expand = (template, m, index, str, groups) =>
            template.replace(/\$(\$|&|`|'|\d{1,2}|<[^>]*>)/g, (whole, k) => {
                if (k === "$") return "$";
                if (k === "&") return m[0];
                if (k === "`") return str.slice(0, index);
                if (k === "'") return str.slice(index + m[0].length);
                if (k[0] === "<") return groups ? groups[k.slice(1, -1)] ?? "" : whole;
                let n = +k;
                if (n >= m.length && k.length === 2 && +k[0] > 0 && +k[0] < m.length) return (m[+k[0]] ?? "") + k[1];
                return n > 0 && n < m.length ? m[n] ?? "" : whole;
            });

        RegExp.prototype[Symbol.replace] = function (str, replacement) {
            // Small inputs are fast natively; only large modules need the detour.
            if (typeof str !== "string" || str.length < 4096) return nativeReplace.call(this, str, replacement);
            const key = this.source + "/" + this.flags;
            let p = parsed.get(key);
            if (p === undefined) parsed.set(key, (p = parse(this)));
            if (!p) return nativeReplace.call(this, str, replacement);

            p.a.lastIndex = 0;
            const ma = p.a.exec(str);
            if (!ma) return str;
            const from = ma.index + ma[0].length;
            p.b.lastIndex = from + p.gap;
            const mb = p.b.exec(str);
            if (!mb) return str;
            // "." must not cross a line break unless the regex is dotAll; if it
            // would, let the native engine decide.
            if (!p.dotAll && NEWLINE.test(str.slice(from, mb.index))) return nativeReplace.call(this, str, replacement);

            const m = [mb[0], ...ma.slice(1), ...mb.slice(1)];
            const groups = ma.groups || mb.groups ? { ...ma.groups, ...mb.groups } : undefined;
            const out =
                typeof replacement === "function"
                    ? String(replacement(...m, mb.index, str, ...(groups ? [groups] : [])))
                    : expand(String(replacement), m, mb.index, str, groups);
            return str.slice(0, mb.index) + out + str.slice(mb.index + mb[0].length);
        };
    }

    // ---------------------------------------------------------------------
    // WebKit hides `navigator.mediaDevices` from apps that don't declare
    // microphone/camera usage (a bare binary outside the .app bundle), and
    // Vencord reads it at startup. Voice won't work there, but the client will.
    // ---------------------------------------------------------------------
    if (!navigator.mediaDevices) {
        const unsupported = () =>
            Promise.reject(new DOMException("Media capture needs the bundled app", "NotSupportedError"));
        const stub = new EventTarget();
        stub.getUserMedia = unsupported;
        stub.getDisplayMedia = unsupported;
        stub.enumerateDevices = () => Promise.resolve([]);
        Object.defineProperty(navigator, "mediaDevices", { value: stub, configurable: true });
    }

    // ---------------------------------------------------------------------
    // Notifications. WKWebView has no Notification API, so Discord would hide
    // desktop notifications entirely. Forward them to the native side.
    // ---------------------------------------------------------------------
    if (!("Notification" in window)) {
        class Notification extends EventTarget {
            static get permission() {
                return "granted";
            }
            static requestPermission(cb) {
                if (typeof cb === "function") cb("granted");
                return Promise.resolve("granted");
            }
            constructor(title, options = {}) {
                super();
                this.title = String(title);
                this.body = String(options.body ?? "");
                this.tag = options.tag ?? "";
                this.icon = options.icon ?? "";
                this.data = options.data ?? null;
                this.onclick = this.onclose = this.onerror = this.onshow = null;
                post({ t: "notify", title: this.title, body: this.body });
                queueMicrotask(() => {
                    const ev = new Event("show");
                    this.onshow?.(ev);
                    this.dispatchEvent(ev);
                });
            }
            close() {}
        }
        window.Notification = Notification;
    }

    // ---------------------------------------------------------------------
    // Themes and QuickCSS. Vencord loads themes with `@import url(...)`, which
    // Discord's Content-Security-Policy blocks for third-party hosts and blob
    // URLs. The webview can't strip response headers, so instead the Rust side
    // downloads every import (recursively) and we apply the result inline.
    // ---------------------------------------------------------------------
    const HOOKED = new Set(["vencord-themes", "vencord-custom-css"]);
    const HAS_IMPORT = /@import\s/i;
    const BLOB_IMPORT = /@import\s+url\(\s*["']?(blob:[^"')\s]+)["']?\s*\)\s*;?/gi;
    const nodeText = Object.getOwnPropertyDescriptor(Node.prototype, "textContent");
    const blobCss = new Map();
    const hooked = new WeakMap();

    const createObjectURL = URL.createObjectURL;
    const revokeObjectURL = URL.revokeObjectURL;
    URL.createObjectURL = function (obj) {
        const url = createObjectURL.call(URL, obj);
        if (obj instanceof Blob && obj.type === "text/css") blobCss.set(url, obj);
        return url;
    };
    URL.revokeObjectURL = function (url) {
        blobCss.delete(url);
        return revokeObjectURL.call(URL, url);
    };

    // Local (uploaded) themes arrive as blob URLs; read them straight from the
    // Blob so no request is needed at all.
    const expandBlobs = async css => {
        const urls = new Set();
        for (const m of css.matchAll(BLOB_IMPORT)) urls.add(m[1]);
        if (!urls.size) return css;
        const texts = new Map();
        await Promise.all(
            [...urls].map(async url => {
                const blob = blobCss.get(url);
                texts.set(url, blob ? await blob.text() : "/* oxicord: theme blob expired */");
            })
        );
        // A local theme may itself start with imports, and imports are only
        // legal at the top of a sheet, so hoist nothing: Rust resolves them
        // in place wherever a prologue begins.
        return css.replace(BLOB_IMPORT, (_, url) => `/*oxicord:sheet*/${texts.get(url)}/*oxicord:end*/`);
    };

    const setHooked = (el, value) => {
        const source = String(value ?? "");
        let st = hooked.get(el);
        if (!st) hooked.set(el, (st = { gen: 0, source: "", call: 0 }));
        st.source = source;
        const gen = ++st.gen;
        if (st.call) {
            pending.delete(st.call);
            st.call = 0;
        }
        const apply = css => {
            if (gen === st.gen) nodeText.set.call(el, css);
        };
        if (!HAS_IMPORT.test(source)) return apply(source);
        expandBlobs(source).then(css => {
            if (gen !== st.gen) return;
            if (!HAS_IMPORT.test(css)) return apply(css);
            st.call = call("css", { css }, apply);
        });
    };

    Object.defineProperty(HTMLStyleElement.prototype, "textContent", {
        configurable: true,
        enumerable: true,
        get() {
            const st = HOOKED.has(this.id) ? hooked.get(this) : undefined;
            return st ? st.source : nodeText.get.call(this);
        },
        set(value) {
            if (HOOKED.has(this.id)) setHooked(this, value);
            else nodeText.set.call(this, value);
        },
    });
})();
