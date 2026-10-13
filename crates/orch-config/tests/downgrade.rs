mod common;

use common::Fixture;

#[test]
fn config_written_for_the_sandbox_release_still_loads() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults]\nsandbox = false\n\n[defaults.presets.open]\nmode = \"auto\"\nsandbox = false\n\n\
         [repos.{}.sandbox]\nwritable = [\"~/.cache\"]\n",
        fx.repo_key()
    ));
    fx.repo_file(
        "[sandbox]\nreadable = [\"/opt/sdk\"]\nhidden = [\"~/.cargo\"]\n\n\
         [presets.boxed]\nmode = \"plan\"\nsandbox = true\n",
    );

    let config = fx.untrusted();

    assert!(config.presets().get("open").is_some());
    assert!(config.presets().get("boxed").is_some());
}
