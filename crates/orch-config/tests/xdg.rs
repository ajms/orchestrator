use std::path::PathBuf;

use orch_config::xdg;

#[test]
fn runtime_dir_is_xdg_runtime_dir_when_set() {
    let lookup = |key: &str| (key == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_string());
    assert_eq!(
        xdg::runtime_dir(lookup),
        Some(PathBuf::from("/run/user/1000"))
    );
}

#[test]
fn runtime_dir_has_no_home_fallback() {
    let lookup = |key: &str| match key {
        "XDG_RUNTIME_DIR" => Some(String::new()),
        "HOME" => Some("/home/u".into()),
        _ => None,
    };
    assert_eq!(xdg::runtime_dir(lookup), None);
}
