use crate::SubagentId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    pub id: SubagentId,
    pub agent_type: String,
    pub description: String,
    pub tool_count: u32,
    pub done: bool,
}
