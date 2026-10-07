use std::collections::HashMap;
use std::path::PathBuf;

use orch_core::{AgentEvent, ConversationId, SubagentId};
use serde::Deserialize;
use serde_json::Value;

use super::{Antigravity, GUARD_EVENT, transcript};
use crate::hook_event::HOOK_EVENT_FIELD;
use crate::{AgentAdapter, ConversationTree, PayloadError};

const INVOKE_TOOL: &str = "invoke_subagent";
const FALLBACK_TYPE: &str = "subagent";

pub(super) struct AntigravityTree {
    agy: Antigravity,
    root: Option<String>,
    pending: Vec<Spec>,
    children: HashMap<String, Child>,
}

struct Child {
    agent_type: String,
    description: String,
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
    pub(super) fn new(agy: Antigravity) -> Self {
        Self {
            agy,
            root: None,
            pending: Vec::new(),
            children: HashMap::new(),
        }
    }

    fn child_events(
        &mut self,
        id: String,
        hook: &Hook,
        event: Option<&str>,
        events: Vec<AgentEvent>,
    ) -> Vec<AgentEvent> {
        let subagent = SubagentId(id.clone());
        let mut out = Vec::new();
        if !self.children.get(&id).is_some_and(|child| child.open) {
            let child = match self.children.remove(&id) {
                Some(known) => known,
                None => self.label(hook),
            };
            out.push(AgentEvent::SubagentStarted {
                id: subagent.clone(),
                agent_type: child.agent_type.clone(),
                description: child.description.clone(),
            });
            self.children.insert(
                id.clone(),
                Child {
                    open: true,
                    ..child
                },
            );
        }
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
        out.extend(events.into_iter().map(|event| match event {
            AgentEvent::ToolStarted { tool, .. } => AgentEvent::ToolStarted {
                tool,
                subagent: Some(subagent.clone()),
            },
            AgentEvent::ToolFinished { tool, .. } => AgentEvent::ToolFinished {
                tool,
                subagent: Some(subagent.clone()),
            },
            other => other,
        }));
        out
    }

    fn label(&mut self, hook: &Hook) -> Child {
        let prompt = hook
            .transcript_path
            .as_deref()
            .and_then(transcript::first_prompt)
            .unwrap_or_default();
        let matched = self
            .pending
            .iter()
            .position(|spec| spec.prompt.trim() == prompt);
        let (agent_type, description) = match matched {
            Some(at) => {
                let spec = self.pending.remove(at);
                (spec.type_name, spec.role)
            }
            None => (
                FALLBACK_TYPE.into(),
                prompt.lines().next().unwrap_or_default().into(),
            ),
        };
        Child {
            agent_type,
            description,
            open: false,
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
        let events = self.agy.map_hook(payload)?;
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return Ok(events);
        };
        let event = value[HOOK_EVENT_FIELD].as_str().map(String::from);
        let mut hook: Hook = serde_json::from_value(value).unwrap_or_default();
        let id = std::mem::take(&mut hook.conversation_id);
        if id.is_empty() {
            return Ok(events);
        }
        if event.as_deref() == Some(GUARD_EVENT)
            && let Some(call) = hook.tool_call.take()
            && call.name == INVOKE_TOOL
        {
            self.pending.extend(call.args.subagents);
        }
        match &self.root {
            None => {
                self.root = Some(id.clone());
                let changed = AgentEvent::ConversationChanged {
                    id: ConversationId(id),
                };
                Ok(std::iter::once(changed).chain(events).collect())
            }
            Some(root) if *root == id => Ok(events),
            Some(_) => Ok(self.child_events(id, &hook, event.as_deref(), events)),
        }
    }

    fn tap(&mut self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        let events = self.agy.map_tap(payload)?;
        for event in &events {
            if let AgentEvent::ConversationChanged { id } = event {
                self.root = Some(id.as_str().into());
            }
        }
        Ok(events)
    }
}
