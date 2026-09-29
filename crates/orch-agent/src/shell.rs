pub(crate) fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}
