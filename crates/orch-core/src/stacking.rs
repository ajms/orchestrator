use crate::SessionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retarget {
    pub session: SessionId,
    pub base: String,
}

pub fn retarget<'a>(
    sessions: impl IntoIterator<Item = (&'a SessionId, &'a str)>,
    landed_branch: &str,
    landed_base: &str,
) -> Vec<Retarget> {
    sessions
        .into_iter()
        .filter(|(_, base)| *base == landed_branch)
        .map(|(session, _)| Retarget {
            session: session.clone(),
            base: landed_base.to_string(),
        })
        .collect()
}
