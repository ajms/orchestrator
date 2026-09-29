use std::path::Path;

use orch_store::Store;

#[test]
fn state_database_follows_xdg_state_home_then_home() {
    let xdg = Store::default_path_from_vars(|key| match key {
        "XDG_STATE_HOME" => Some("/x/state".into()),
        "HOME" => Some("/home/u".into()),
        _ => None,
    });
    assert_eq!(
        xdg.as_deref(),
        Some(Path::new("/x/state/orchestrator/state.db"))
    );

    let home = Store::default_path_from_vars(|key| (key == "HOME").then(|| "/home/u".into()));
    assert_eq!(
        home.as_deref(),
        Some(Path::new("/home/u/.local/state/orchestrator/state.db"))
    );
}

#[test]
fn opening_creates_the_state_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nested/orchestrator/state.db");
    Store::open(&path).unwrap();
    assert!(path.is_file());
}

#[test]
fn a_state_database_from_a_newer_orch_is_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 999).unwrap();
    drop(conn);
    let err = Store::open(&path)
        .err()
        .expect("newer schema must be refused");
    assert!(
        matches!(err, orch_store::StoreError::NewerSchema { found: 999, .. }),
        "{err}"
    );
}
