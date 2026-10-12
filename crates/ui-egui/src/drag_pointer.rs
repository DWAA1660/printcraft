//! Where the pointer is while files are dragged over the window. The windowing layer reports
//! that a file hovers, but not where, and sends no pointer moves until the drag ends; the
//! system has to be asked.

/// The pointer in this window's coordinates (egui points), when the system can tell.
#[cfg(target_os = "macos")]
pub(crate) fn in_window(ctx: &egui::Context) -> Option<egui::Pos2> {
    use objc2_app_kit::{NSEvent, NSScreen};
    // AppKit's screen list is only for the main thread (where egui runs).
    let mtm = objc2::MainThreadMarker::new()?;
    // Screen coordinates start at the bottom-left of the first screen; windows at their top-left.
    let top = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let pointer = NSEvent::mouseLocation();
    let inner = ctx.input(|i| i.viewport().inner_rect)?;
    // egui's points are the system's, divided by the zoom factor.
    let zoom = ctx.zoom_factor().max(0.01) as f64;
    let (x, y) = (pointer.x / zoom, (top - pointer.y) / zoom);
    let pos = egui::pos2(x as f32 - inner.min.x, y as f32 - inner.min.y);
    (pos.x.is_finite() && pos.y.is_finite()).then_some(pos)
}

/// Ask X11 through a fallible Rust connection. Native Wayland cannot expose a global pointer;
/// missing displays, authentication failures and failed queries all leave egui's input alone.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn in_window(ctx: &egui::Context) -> Option<egui::Pos2> {
    x11_pointer_in_window(ctx, None)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn x11_pointer_in_window(ctx: &egui::Context, display: Option<&str>) -> Option<egui::Pos2> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    let (connection, screen) = x11rb::connect(display).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let pointer = connection.query_pointer(root).ok()?.reply().ok()?;
    if !pointer.same_screen {
        return None;
    }
    let inner = ctx.input(|i| i.viewport().inner_rect)?;
    screen_to_window([f32::from(pointer.root_x), f32::from(pointer.root_y)], ctx.pixels_per_point(), inner.min)
}

/// Windows reports screen pixels; egui wants points relative to the window's top-left.
#[cfg(windows)]
pub(crate) fn in_window(ctx: &egui::Context) -> Option<egui::Pos2> {
    use mouse_position::mouse_position::Mouse;
    // Asks the system for the pointer, since winit sends none during a file drag. It fails where
    // there is no X11 or Win32 to ask (Wayland): no position, so the caller leaves egui's as is.
    let Mouse::Position { x, y } = Mouse::get_mouse_position() else {
        return None;
    };
    // The pointer is in screen coordinates; the window's own origin is needed to make it relative.
    let inner = ctx.input(|i| i.viewport().inner_rect)?;

    // The system gives pixels but `inner_rect` is in egui points, so convert before subtracting
    // (otherwise the position is off on HiDPI screens).
    screen_to_window([x as f32, y as f32], ctx.pixels_per_point(), inner.min)
}

#[cfg(any(target_os = "linux", target_os = "freebsd", windows, test))]
fn screen_to_window([x, y]: [f32; 2], scale: f32, origin: egui::Pos2) -> Option<egui::Pos2> {
    let scale = scale.max(0.01);
    let pos = egui::pos2(x / scale - origin.x, y / scale - origin.y);
    // Never hand egui a NaN or infinite position.
    (pos.x.is_finite() && pos.y.is_finite()).then_some(pos)
}

// Targets with no way to ask (wasm, other Unixes): no position, as before.
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
pub(crate) fn in_window(_ctx: &egui::Context) -> Option<egui::Pos2> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_pixels_project_to_window_points() {
        assert_eq!(screen_to_window([300.0, 240.0], 2.0, egui::pos2(100.0, 80.0)), Some(egui::pos2(50.0, 40.0)));
        assert_eq!(screen_to_window([-80.0, 30.0], 1.0, egui::pos2(-100.0, 10.0)), Some(egui::pos2(20.0, 20.0)));
        assert!(screen_to_window([f32::NAN, 20.0], 1.0, egui::Pos2::ZERO).is_none());
        assert!(screen_to_window([20.0, f32::INFINITY], 1.0, egui::Pos2::ZERO).is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn unavailable_x11_pointer_is_a_missing_position() {
        // An absent local Unix socket, without changing the shared process environment.
        assert!(x11_pointer_in_window(&egui::Context::default(), Some("unix/:65535")).is_none());
    }
}
