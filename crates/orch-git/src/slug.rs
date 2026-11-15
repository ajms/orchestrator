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

const ADJECTIVES: &[&str] = &[
    "Amber", "Bold", "Brisk", "Calm", "Clever", "Cosmic", "Crisp", "Dapper", "Eager", "Fancy",
    "Gentle", "Golden", "Happy", "Hidden", "Jolly", "Keen", "Lucky", "Merry", "Mellow", "Nimble",
    "Noble", "Plucky", "Quiet", "Rapid", "Rustic", "Silent", "Silver", "Sleepy", "Spry", "Steady",
    "Sunny", "Swift", "Tidy", "Velvet", "Vivid", "Witty",
];

const NOUNS: &[&str] = &[
    "Badger", "Beacon", "Comet", "Cedar", "Falcon", "Fern", "Finch", "Fox", "Glacier", "Harbor",
    "Heron", "Lantern", "Lynx", "Maple", "Meadow", "Moose", "Otter", "Owl", "Panda", "Pebble",
    "Pine", "Quokka", "Raven", "River", "Robin", "Sparrow", "Summit", "Thistle", "Tiger", "Walrus",
    "Willow", "Wren",
];

pub fn random_title() -> String {
    use std::hash::{BuildHasher, Hasher};
    let seed = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as usize;
    let adjective = ADJECTIVES[seed % ADJECTIVES.len()];
    let noun = NOUNS[(seed / ADJECTIVES.len()) % NOUNS.len()];
    format!("{adjective} {noun}")
}
