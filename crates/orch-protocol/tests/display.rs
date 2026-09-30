use orch_protocol::DisplayVars;

fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
    |key| {
        vars.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn the_display_comes_from_wayland_display_and_display() {
    let display =
        DisplayVars::from_vars(env(&[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":0")]));

    assert_eq!(
        display,
        DisplayVars {
            wayland_display: Some("wayland-1".into()),
            x11_display: Some(":0".into()),
        }
    );
}

#[test]
fn an_empty_display_variable_counts_as_unset() {
    let display = DisplayVars::from_vars(env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", "")]));

    assert_eq!(display, DisplayVars::default());
}
