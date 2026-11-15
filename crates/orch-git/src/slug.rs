const MAX_WORDS: usize = 6;
const MAX_LEN: usize = 40;

pub fn slugify(prompt: &str) -> String {
    let mut slug = String::new();
    let words = prompt
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(MAX_WORDS);
    for word in words {
        let separator = usize::from(!slug.is_empty());
        if slug.len() + separator + word.len() > MAX_LEN {
            if slug.is_empty() {
                slug.push_str(&word[..MAX_LEN].to_ascii_lowercase());
            }
            break;
        }
        if separator == 1 {
            slug.push('-');
        }
        slug.push_str(&word.to_ascii_lowercase());
    }
    if slug.is_empty() {
        slug.push_str("session");
    }
    slug
}
