use orch_git::slugify;

#[test]
fn slug_is_lowercase_words_joined_by_hyphens() {
    assert_eq!(slugify("Fix the Login bug!"), "fix-the-login-bug");
}

#[test]
fn slug_drops_punctuation_and_non_ascii_runs() {
    assert_eq!(
        slugify("  Add `--json` flag (CLI) — über fast  "),
        "add-json-flag-cli-ber-fast"
    );
}

#[test]
fn slug_keeps_only_the_first_few_words() {
    assert_eq!(
        slugify("refactor the session store so that it uses sqlite transactions everywhere"),
        "refactor-the-session-store-so-that"
    );
}

#[test]
fn slug_never_exceeds_forty_characters() {
    let slug = slugify("internationalization accessibility documentation reorganization");
    assert!(slug.len() <= 40, "{slug}");
    assert!(!slug.ends_with('-'), "{slug}");
}

#[test]
fn slug_of_a_prompt_without_words_is_session() {
    assert_eq!(slugify("?!…"), "session");
}
