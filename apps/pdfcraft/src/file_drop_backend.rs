//! File-drop backend selection, with an injectable X11 connection probe.

pub(crate) fn should_prefer_x11(override_set: bool, wayland_set: bool, display_set: bool, probe: impl FnOnce() -> bool) -> bool {
    !override_set && wayland_set && display_set && probe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_x11_keeps_wayland_startup_available() {
        assert!(!should_prefer_x11(false, true, true, || false));
    }

    #[test]
    fn reachable_x11_enables_file_drops() {
        assert!(should_prefer_x11(false, true, true, || true));
    }

    #[test]
    fn overrides_and_missing_displays_do_not_probe() {
        for (override_set, wayland_set, display_set) in [(true, true, true), (false, false, true), (false, true, false)] {
            assert!(!should_prefer_x11(override_set, wayland_set, display_set, || panic!("no connection needed")));
        }
    }
}
