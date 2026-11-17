// Oxicord postamble. Runs right after the Vencord bundle.
(() => {
    "use strict";
    const ox = window.__oxicord;
    if (!ox) return;
    const CORE_CSS = __VENCORD_CSS__;

    // Chromium (Windows) runs this before the page has a root element; WebKit creates it first. Wait for it.
    const onRoot = fn => {
        if (document.documentElement) return fn();
        const mo = new MutationObserver(() => {
            if (!document.documentElement) return;
            mo.disconnect();
            fn();
        });
        mo.observe(document, { childList: true });
    };

    // Vencord's plain web build expects its host to supply the core stylesheet
    // (the extension passes a URL; nothing does here), so fill it in directly.
    const fillCoreCss = () => {
        let el = document.getElementById("vencord-css-core");
        if (!el) {
            el = document.createElement("style");
            el.id = "vencord-css-core";
            document.documentElement.append(el);
        }
        if (!el.textContent) el.textContent = CORE_CSS;
    };

    // The web build's QuickCSS editor is a popup that loads Monaco from an
    // extension URL, which doesn't exist here. Provide a small in-page editor.
    const installEditor = () => {
        const native = window.VencordNative;
        if (!native?.quickCss) return;
        let panel = null;
        native.quickCss.openEditor = async () => {
            if (panel) return panel.querySelector("textarea").focus();
            panel = document.createElement("div");
            panel.style.cssText =
                "position:fixed;right:24px;bottom:24px;width:min(560px,80vw);height:min(420px,70vh);" +
                "z-index:2147483647;display:flex;flex-direction:column;background:#1e1f22;color:#dbdee1;" +
                "border:1px solid #3f4147;border-radius:8px;box-shadow:0 8px 32px rgba(0,0,0,.5);overflow:hidden;" +
                "resize:both;font:13px system-ui";
            const bar = document.createElement("div");
            bar.style.cssText =
                "display:flex;align-items:center;justify-content:space-between;padding:6px 10px;background:#2b2d31;cursor:move;user-select:none";
            bar.textContent = "QuickCSS";
            const close = document.createElement("button");
            close.textContent = "Close";
            close.style.cssText = "background:none;border:0;color:inherit;cursor:pointer;font:inherit";
            bar.append(close);
            const area = document.createElement("textarea");
            area.spellcheck = false;
            area.style.cssText =
                "flex:1;resize:none;border:0;outline:0;padding:10px;background:transparent;color:inherit;" +
                "font:12px ui-monospace,Menlo,Consolas,monospace;tab-size:4;white-space:pre";
            area.value = await native.quickCss.get();
            let timer = 0;
            area.addEventListener("input", () => {
                clearTimeout(timer);
                timer = setTimeout(() => native.quickCss.set(area.value), 250);
            });
            area.addEventListener("keydown", e => {
                e.stopPropagation();
                if (e.key !== "Tab") return;
                e.preventDefault();
                area.setRangeText("    ", area.selectionStart, area.selectionEnd, "end");
                area.dispatchEvent(new Event("input"));
            });
            close.addEventListener("click", () => {
                clearTimeout(timer);
                native.quickCss.set(area.value);
                panel.remove();
                panel = null;
            });
            bar.addEventListener("pointerdown", e => {
                if (e.target === close) return;
                const rect = panel.getBoundingClientRect();
                const dx = e.clientX - rect.left;
                const dy = e.clientY - rect.top;
                const move = ev => {
                    panel.style.left = `${Math.max(0, ev.clientX - dx)}px`;
                    panel.style.top = `${Math.max(0, ev.clientY - dy)}px`;
                    panel.style.right = panel.style.bottom = "auto";
                };
                const up = () => {
                    removeEventListener("pointermove", move);
                    removeEventListener("pointerup", up);
                };
                addEventListener("pointermove", move);
                addEventListener("pointerup", up);
            });
            panel.append(bar, area);
            document.body.append(panel);
            area.focus();
        };
    };

    // The built-in glass skin. It goes in first so Vencord themes and QuickCSS,
    // which load later, can still override it.
    const skin = document.createElement("style");
    skin.id = "oxicord-skin";
    skin.textContent = __OXICORD_SKIN_CSS__;
    onRoot(() => document.documentElement.append(skin));
    // An attribute, not a class: Discord rewrites the class list as it boots.
    onRoot(() => {
        document.documentElement.toggleAttribute("data-ox-glass", __OXICORD_SKIN_ON__);
        document.documentElement.toggleAttribute("data-ox-inset", __OXICORD_INSET__);
    });

    // Discord takes a couple of seconds to boot, and its own loading screen is
    // invisible on a see-through window. Show the finished dark glass with a
    // quiet spinner straight away, and lift it once the app has drawn.
    const splash = document.createElement("div");
    splash.id = "oxicord-splash";
    splash.innerHTML = "<i></i>";
    onRoot(() => document.documentElement.append(splash));
    const booted = () =>
        document.querySelector('nav[class*="guilds_"]') ||
        (/^\/(login|register|invite|oauth2|verify|reset)/.test(location.pathname) &&
            document.querySelector("#app-mount form"));
    const lift = () => {
        watcher.disconnect();
        clearTimeout(giveUp);
        ox.post({ t: "interactive", ms: Math.round(performance.now() - ox.t0) });
        // Those pages are hidden from the sidebar, so don't reopen on one.
        if (/^\/(quest-home|shop|store)\b/.test(location.pathname)) {
            history.pushState(null, "", "/channels/@me");
            dispatchEvent(new PopStateEvent("popstate"));
        }
        splash.classList.add("done");
        setTimeout(() => splash.remove(), 220);
    };
    const watcher = new MutationObserver(() => booted() && lift());
    watcher.observe(document, { childList: true, subtree: true });
    const giveUp = setTimeout(lift, 15000);

    // With no native title strip, Discord's top bar moves the window: drag on
    // its empty space, double-click to zoom. Its buttons keep working.
    if (__OXICORD_INSET__) {
        document.addEventListener(
            "mousedown",
            e => {
                if (e.button !== 0 || e.clientY > 44 || !(e.target instanceof Element)) return;
                const bar = e.target.closest('div[class^="bar_"]');
                if (!bar || !bar.querySelector(':scope > [class^="title_"]')) return;
                if (e.target.closest('button, a, input, [role="button"], [class*="leading_"], [class*="trailing_"]')) return;
                e.preventDefault();
                ox.post({ t: e.detail >= 2 ? "zoom-window" : "drag" });
            },
            true
        );
    }

    installEditor();

    // Registered after Vencord's own DOMContentLoaded listener, so its style
    // root is already attached by the time this runs.
    document.addEventListener(
        "DOMContentLoaded",
        () => {
            fillCoreCss();
            const v = window.Vencord;
            ox.post({
                t: "ready",
                vencord: typeof v === "object" && v !== null,
                plugins: v?.Plugins?.plugins ? Object.keys(v.Plugins.plugins).length : 0,
                css: (document.getElementById("vencord-css-core")?.textContent ?? "").length,
                skin: document.documentElement.hasAttribute("data-ox-glass"),
                ms: Math.round(performance.now() - ox.t0),
            });
        },
        { once: true }
    );

    // A second look once Discord itself has booted: is the skin really applied?
    setTimeout(() => {
        const root = document.documentElement;
        const cs = getComputedStyle(root);
        ox.post({
            t: "probe",
            vencord: typeof window.Vencord === "object" && window.Vencord !== null,
            plugins: window.Vencord?.Plugins?.plugins ? Object.keys(window.Vencord.Plugins.plugins).length : 0,
            core: (document.getElementById("vencord-css-core")?.textContent ?? "").length,
            skin: root.hasAttribute("data-ox-glass"),
            classes: root.className,
            sidebar: cs.getPropertyValue("--background-base-lower").trim(),
            pageBg: cs.backgroundColor,
            bodyBg: getComputedStyle(document.body).backgroundColor,
            title: document.title,
            path: location.pathname.split("/").slice(0, 2).join("/"),
        });
    }, 5000);
})();
