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
