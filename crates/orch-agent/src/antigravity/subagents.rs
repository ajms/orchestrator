use std::collections::HashMap;
use std::path::PathBuf;

use orch_core::{AgentEvent, ConversationId, SubagentId};
use serde::Deserialize;

use super::{GUARD_EVENT, hooks, transcript};
use crate::hook_event::HOOK_EVENT_FIELD;
use crate::{ConversationTree, PayloadError};

const INVOKE_TOOL: &str = "invoke_subagent";
const FALLBACK_TYPE: &str = "subagent";

#[derive(Default)]
pub(super) struct AntigravityTree {
    root: Option<String>,
    pending: Vec<Spec>,
    children: HashMap<String, Child>,
}

struct Child {
    agent_type: String,
    description: String,
    matched: bool,
    open: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Hook {
    conversation_id: String,
    transcript_path: Option<PathBuf>,
    tool_call: Option<ToolCall>,
    fully_idle: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolCall {
    name: String,
    args: Args,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Args {
    #[serde(rename = "Subagents")]
    subagents: Vec<Spec>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Spec {
    #[serde(rename = "TypeName")]
    type_name: String,
    #[serde(rename = "Role")]
    role: String,
    #[serde(rename = "Prompt")]
    prompt: String,
}

impl AntigravityTree {
    fn child_events(
        &mut self,
        id: String,
        hook: &Hook,
        event: Option<&str>,
        events: Vec<AgentEvent>,
    ) -> Vec<AgentEvent> {
        let subagent = SubagentId(id.clone());
        let mut out = Vec::new();
        let (child, relabelled) = match self.children.remove(&id) {
            Some(known) if self.settled(&known) => (known, false),
            Some(known) => {
                let child = Child {
                    open: known.open,
                    ..self.label(hook)
                };
                let relabelled =
                    child.agent_type != known.agent_type || child.description != known.description;
                (child, relabelled)
            }
            None => (self.label(hook), false),
        };
        if !child.open || relabelled {
            out.push(AgentEvent::SubagentStarted {
                id: subagent.clone(),
                agent_type: child.agent_type.clone(),
                description: child.description.clone(),
            });
        }
        self.children.insert(
            id.clone(),
            Child {
                open: true,
                ..child
            },
        );
        if event == Some("Stop") {
            if hook.fully_idle {
                out.push(AgentEvent::TurnEnded);
            }
            out.push(AgentEvent::SubagentFinished { id: subagent });
            if let Some(child) = self.children.get_mut(&id) {
                child.open = false;
            }
            return out;
        }
        out.extend(events.into_iter().filter_map(|event| match event {
            AgentEvent::PromptSubmitted => None,
            AgentEvent::ToolStarted { tool, .. } => Some(AgentEvent::ToolStarted {
                tool,
                subagent: Some(subagent.clone()),
            }),
            AgentEvent::ToolFinished { tool, .. } => Some(AgentEvent::ToolFinished {
                tool,
                subagent: Some(subagent.clone()),
            }),
            other => Some(other),
        }));
        out
    }

    fn settled(&self, child: &Child) -> bool {
        child.matched || (self.pending.is_empty() && !child.description.is_empty())
    }

    fn label(&mut self, hook: &Hook) -> Child {
        let prompt = hook
            .transcript_path
            .as_deref()
            .map(transcript::full_transcript)
            .and_then(|path| transcript::first_prompt(&path))
            .unwrap_or_default();
        let matched = self
            .pending
            .iter()
            .position(|spec| spec.prompt.trim() == prompt);
        match matched {
            Some(at) => {
                let spec = self.pending.remove(at);
                Child {
                    agent_type: spec.type_name,
                    description: spec.role,
                    matched: true,
                    open: false,
                }
            }
            None => Child {
                agent_type: FALLBACK_TYPE.into(),
                description: prompt.lines().next().unwrap_or_default().into(),
                matched: false,
                open: false,
            },
        }
    }
}

impl ConversationTree for AntigravityTree {
    fn restart(&mut self, conversation: Option<&ConversationId>) {
        self.root = conversation.map(|id| id.as_str().into());
        self.pending.clear();
        for child in self.children.values_mut() {
            child.open = false;
        }
    }

    fn hook(&mut self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        let value = hooks::json(payload)?;
        let events = hooks::map_value(&value)?;
        let event = value[HOOK_EVENT_FIELD].as_str();
        let mut hook = Hook::deserialize(&value).unwrap_or_default();
        let id = std::mem::take(&mut hook.conversation_id);
        if id.is_empty() {
            return Ok(events);
        }
        if event == Some(GUARD_EVENT)
            && let Some(call) = hook.tool_call.take()
            && call.name == INVOKE_TOOL
        {
            self.pending.extend(call.args.subagents);
        }
        match &self.root {
            Some(root) if *root == id => Ok(events),
            None if !self.children.contains_key(&id) => {
                self.root = Some(id.clone());
                let changed = AgentEvent::ConversationChanged {
                    id: ConversationId(id),
                };
                Ok(std::iter::once(changed).chain(events).collect())
            }
            _ => Ok(self.child_events(id, &hook, event, events)),
        }
    }

    fn tap(&mut self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        let events = super::statusline::map_tap(payload)?;
        for event in &events {
            if let AgentEvent::ConversationChanged { id } = event {
                self.root = Some(id.as_str().into());
            }
        }
        Ok(events)
    }
}
