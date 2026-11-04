use crate::error::ConfigProblem;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadableOwners(Vec<String>);

pub fn is_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && !owner.starts_with('-')
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl ReadableOwners {
    pub fn new(owners: Vec<String>) -> Result<Self, ConfigProblem> {
        match owners.iter().find(|owner| !is_github_owner(owner)) {
            Some(owner) => Err(ConfigProblem::ReadableOwner(owner.clone())),
            None => Ok(ReadableOwners(owners)),
        }
    }

    pub fn reads(&self, owner: &str) -> bool {
        self.0
            .iter()
            .any(|listed| listed.eq_ignore_ascii_case(owner))
    }

    pub fn spellings(&self) -> Vec<String> {
        let mut spellings: Vec<String> = Vec::new();
        for owner in &self.0 {
            for spelling in [owner.clone(), owner.to_ascii_lowercase()] {
                if !spellings.contains(&spelling) {
                    spellings.push(spelling);
                }
            }
        }
        spellings
    }

    pub fn lower_case(&self) -> Vec<String> {
        let mut lower: Vec<String> = Vec::new();
        for owner in self.0.iter().map(|owner| owner.to_ascii_lowercase()) {
            if !lower.contains(&owner) {
                lower.push(owner);
            }
        }
        lower
    }
}
