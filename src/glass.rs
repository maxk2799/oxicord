//! The blur behind the see-through window (macOS).
//!
//! This has to run after the webview is built: wry installs its own content
//! view, which discards anything attached to the window before that.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
};
use tao::platform::macos::WindowExtMacOS;
use tao::window::Window;

pub fn apply(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: tao hands out its own live NSView, and we are on the main thread.
    let view: &NSView = unsafe { &*(window.ns_view() as *const NSView) };
    let Some(content) = view.window().and_then(|w| w.contentView()) else { return };

    let blur = NSVisualEffectView::initWithFrame(mtm.alloc(), content.bounds());
    // The darkest, most heavily blurred material, so whatever is behind the
    // window never competes with the text in front of it.
    blur.setMaterial(NSVisualEffectMaterial::HUDWindow);
    blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    blur.setState(NSVisualEffectState::Active);
    blur.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content.addSubview_positioned_relativeTo(&blur, NSWindowOrderingMode::Below, None);
}
