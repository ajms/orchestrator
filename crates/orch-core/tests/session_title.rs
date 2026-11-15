use orch_core::{chosen_title, random_title};

#[test]
fn a_random_title_is_two_capitalised_words() {
    let title = random_title();
    let words: Vec<&str> = title.split(' ').collect();
    assert_eq!(words.len(), 2, "{title}");
    assert!(
        words
            .iter()
            .all(|word| word.starts_with(|c: char| c.is_ascii_uppercase())
                && word.chars().all(|c| c.is_ascii_alphabetic())),
        "{title}"
    );
}

#[test]
fn random_titles_vary() {
    let titles: std::collections::HashSet<String> = (0..20).map(|_| random_title()).collect();
    assert!(titles.len() > 1, "{titles:?}");
}

#[test]
fn a_given_title_is_chosen_trimmed() {
    assert_eq!(chosen_title(Some("  Login bug ")), Some("Login bug"));
}

#[test]
fn a_missing_or_blank_title_is_not_chosen() {
    assert_eq!(chosen_title(None), None);
    assert_eq!(chosen_title(Some("   ")), None);
}
