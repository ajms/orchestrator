use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

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
    let seed = random_seed();
    let adjective = ADJECTIVES[seed % ADJECTIVES.len()];
    let noun = NOUNS[(seed / ADJECTIVES.len()) % NOUNS.len()];
    format!("{adjective} {noun}")
}

/// The title a Session is created with, if one was given.
pub fn chosen_title(title: Option<&str>) -> Option<&str> {
    title.map(str::trim).filter(|title| !title.is_empty())
}

fn random_seed() -> usize {
    RandomState::new().build_hasher().finish() as usize
}
