// Oxicord postamble. Runs right after the Vencord bundle.
(() => {
    "use strict";
    const ox = window.__oxicord;
    if (!ox) return;
    const CORE_CSS = __VENCORD_CSS__;

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
                ms: Math.round(performance.now() - ox.t0),
            });
        },
        { once: true }
    );
})();
