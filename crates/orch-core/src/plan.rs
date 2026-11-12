//! A Session's Plan: the ordered Tasks its Agent is working through.
//!
//! Only the Agent writes it, through `orch plan` in the Sandbox, so every
//! [`PlanOp`] is untrusted input and [`Plan::apply`] validates it.

use serde::{Deserialize, Serialize};

pub type TaskId = u32;

pub const MAX_TASKS: usize = 200;
pub const MAX_TITLE: usize = 200;
pub const MAX_LINK: usize = 500;
pub const MAX_BLOCKERS: usize = 50;
pub const MAX_ATTACHMENTS: usize = 50;
pub const MAX_ATTACHMENT_ID: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Dropped,
}

impl TaskStatus {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "in-progress" | "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            "dropped" => Some(Self::Dropped),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in-progress",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }

    /// Whether a Task in this status no longer blocks the Tasks it blocks.
    pub fn settled(self) -> bool {
        matches!(self, Self::Done | Self::Dropped)
    }
}

/// What works on a Task: a Subagent or a Child Session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Attachment {
    Subagent(String),
    ChildSession(String),
}

impl Attachment {
    pub fn id(&self) -> &str {
        match self {
            Attachment::Subagent(id) | Attachment::ChildSession(id) => id,
        }
    }

    pub fn is_child_session(&self) -> bool {
        matches!(self, Attachment::ChildSession(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub blocked_by: Vec<TaskId>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// Ids are never reused, so a stale reference can't land on a new Task.
    #[serde(default)]
    pub next_id: TaskId,
    /// Set when the stored Plan couldn't be decoded: the reason it can't be
    /// read. Such a Plan is shown as unreadable and never written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
}

/// Where an unsettled Task stands against its blockers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Done or Dropped.
    Settled,
    Ready,
    BlockedBy(Vec<TaskId>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanOp {
    Show,
    Add {
        title: String,
        #[serde(default)]
        after: Option<TaskId>,
        #[serde(default)]
        blocked_by: Vec<TaskId>,
        #[serde(default)]
        link: Option<String>,
    },
    Status {
        task: TaskId,
        status: TaskStatus,
    },
    Block {
        task: TaskId,
        by: Vec<TaskId>,
    },
    Unblock {
        task: TaskId,
        by: Vec<TaskId>,
    },
    Link {
        task: TaskId,
        link: Option<String>,
    },
    Attach {
        task: TaskId,
        attachment: Attachment,
    },
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// No Task was ever added: nothing worth storing.
    pub fn is_untouched(&self) -> bool {
        self.tasks.is_empty() && self.next_id == 0 && self.unreadable.is_none()
    }

    /// A Plan standing in for a stored one that couldn't be decoded.
    pub fn unreadable(reason: impl Into<String>) -> Self {
        Plan {
            unreadable: Some(reason.into()),
            ..Plan::default()
        }
    }

    /// Attaches `subagent` to the Task its description names (see
    /// [`task_marker`]), if that Task exists and doesn't have it yet.
    /// Returns whether the Plan changed.
    pub fn attach_named(&mut self, subagent: &str, description: &str) -> bool {
        if self.unreadable.is_some() {
            return false;
        }
        let Some(task) = task_marker(description) else {
            return false;
        };
        let attachment = Attachment::Subagent(subagent.to_string());
        let attached = self
            .task(task)
            .is_none_or(|known| known.attachments.contains(&attachment));
        !attached && self.apply(PlanOp::Attach { task, attachment }).is_ok()
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    /// Tasks Done, out of those not Dropped.
    pub fn progress(&self) -> (usize, usize) {
        let counted = self
            .tasks
            .iter()
            .filter(|task| task.status != TaskStatus::Dropped);
        let (done, total) = counted.fold((0, 0), |(done, total), task| {
            (
                done + usize::from(task.status == TaskStatus::Done),
                total + 1,
            )
        });
        (done, total)
    }

    /// The blockers of `task` that are neither Done nor Dropped.
    pub fn open_blockers(&self, task: &Task) -> Vec<TaskId> {
        task.blocked_by
            .iter()
            .copied()
            .filter(|id| {
                self.task(*id)
                    .is_some_and(|blocker| !blocker.status.settled())
            })
            .collect()
    }

    /// Whether `task` is settled, ready, or blocked (and by which Tasks).
    pub fn readiness(&self, task: &Task) -> Readiness {
        if task.status.settled() {
            return Readiness::Settled;
        }
        let open = self.open_blockers(task);
        match open.is_empty() {
            true => Readiness::Ready,
            false => Readiness::BlockedBy(open),
        }
    }

    /// A Task is ready when every Task it is blocked by is Done or Dropped.
    pub fn is_ready(&self, task: &Task) -> bool {
        self.open_blockers(task).is_empty()
    }

    /// Applies `op`, or says why it was refused and leaves the Plan unchanged.
    /// Returns what was done, for the Agent.
    pub fn apply(&mut self, op: PlanOp) -> Result<String, String> {
        if let Some(reason) = &self.unreadable {
            return Err(format!(
                "this Session's stored Plan can't be read: {reason}"
            ));
        }
        let mut next = self.clone();
        let said = next.apply_in_place(op)?;
        *self = next;
        Ok(said)
    }

    fn apply_in_place(&mut self, op: PlanOp) -> Result<String, String> {
        match op {
            PlanOp::Show => Ok(String::new()),
            PlanOp::Add {
                title,
                after,
                blocked_by,
                link,
            } => {
                if self.tasks.len() >= MAX_TASKS {
                    return Err(format!("a Plan holds at most {MAX_TASKS} Tasks"));
                }
                let title = checked_title(&title)?;
                let link = link.as_deref().map(checked_link).transpose()?;
                let position = match after {
                    None => self.tasks.len(),
                    Some(after) => self.position(after)? + 1,
                };
                let id = self.next_id.max(1);
                self.next_id = id.checked_add(1).ok_or("the Plan ran out of Task ids")?;
                self.tasks.insert(
                    position,
                    Task {
                        id,
                        title,
                        status: TaskStatus::Pending,
                        blocked_by: Vec::new(),
                        attachments: Vec::new(),
                        link,
                    },
                );
                self.block(id, &blocked_by)?;
                Ok(format!("added Task {id}"))
            }
            PlanOp::Status { task, status } => {
                let index = self.position(task)?;
                self.tasks[index].status = status;
                Ok(format!("Task {task} is {}", status.name()))
            }
            PlanOp::Block { task, by } => {
                self.position(task)?;
                self.block(task, &by)?;
                Ok(format!("Task {task} is blocked by {}", ids(&by)))
            }
            PlanOp::Unblock { task, by } => {
                let index = self.position(task)?;
                self.tasks[index]
                    .blocked_by
                    .retain(|blocker| !by.contains(blocker));
                Ok(format!("Task {task} is no longer blocked by {}", ids(&by)))
            }
            PlanOp::Link { task, link } => {
                let index = self.position(task)?;
                let link = link
                    .as_deref()
                    .filter(|link| !link.is_empty())
                    .map(checked_link)
                    .transpose()?;
                let said = match &link {
                    Some(link) => format!("Task {task} links {link}"),
                    None => format!("Task {task} has no link"),
                };
                self.tasks[index].link = link;
                Ok(said)
            }
            PlanOp::Attach { task, attachment } => {
                let index = self.position(task)?;
                checked_attachment_id(attachment.id())?;
                let said = match &attachment {
                    Attachment::Subagent(id) => format!("Subagent {id} works on Task {task}"),
                    Attachment::ChildSession(id) => {
                        format!("Child Session {id} works on Task {task}")
                    }
                };
                let attachments = &mut self.tasks[index].attachments;
                if !attachments.contains(&attachment) {
                    if attachments.len() >= MAX_ATTACHMENTS {
                        return Err(format!(
                            "a Task holds at most {MAX_ATTACHMENTS} attachments"
                        ));
                    }
                    attachments.push(attachment);
                }
                Ok(said)
            }
        }
    }

    fn position(&self, id: TaskId) -> Result<usize, String> {
        self.tasks
            .iter()
            .position(|task| task.id == id)
            .ok_or_else(|| format!("the Plan has no Task {id}"))
    }

    fn block(&mut self, task: TaskId, by: &[TaskId]) -> Result<(), String> {
        for &blocker in by {
            self.position(blocker)?;
            if blocker == task {
                return Err(format!("Task {task} can't be blocked by itself"));
            }
            if self.reaches(blocker, task) {
                return Err(format!(
                    "Task {task} blocked by Task {blocker} would be a cycle"
                ));
            }
            let index = self.position(task)?;
            let blocked_by = &mut self.tasks[index].blocked_by;
            if !blocked_by.contains(&blocker) {
                if blocked_by.len() >= MAX_BLOCKERS {
                    return Err(format!("a Task is blocked by at most {MAX_BLOCKERS} Tasks"));
                }
                blocked_by.push(blocker);
            }
        }
        Ok(())
    }

    /// Whether `from` is (transitively) blocked by `to`.
    fn reaches(&self, from: TaskId, to: TaskId) -> bool {
        let mut seen = Vec::new();
        let mut stack = vec![from];
        while let Some(id) = stack.pop() {
            if id == to {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(task) = self.task(id) {
                stack.extend(task.blocked_by.iter().copied());
            }
        }
        false
    }
}

/// The Task a Subagent's description names with a `[task:N]` marker, the way
/// an Agent names the Task a Subagent serves when it spawns it.
pub fn task_marker(description: &str) -> Option<TaskId> {
    let lower = description.to_ascii_lowercase();
    lower.match_indices("[task:").find_map(|(at, marker)| {
        let rest = &lower[at + marker.len()..];
        let (id, _) = rest.split_once(']')?;
        id.trim().parse().ok()
    })
}

/// Task ids as a comma-separated list: `1, 2, 3`.
pub fn join_ids(ids: &[TaskId]) -> String {
    let ids: Vec<_> = ids.iter().map(ToString::to_string).collect();
    ids.join(", ")
}

fn ids(ids: &[TaskId]) -> String {
    let named: Vec<_> = ids.iter().map(|id| format!("Task {id}")).collect();
    named.join(", ")
}

fn checked_text(text: &str, what: &str, max: usize) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("a {what} can't be empty"));
    }
    if text.chars().count() > max {
        return Err(format!("a {what} is at most {max} characters"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("a {what} can't hold control characters"));
    }
    Ok(text.to_string())
}

fn checked_title(title: &str) -> Result<String, String> {
    checked_text(title, "Task title", MAX_TITLE)
}

fn checked_link(link: &str) -> Result<String, String> {
    checked_text(link, "link", MAX_LINK)
}

fn checked_attachment_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ATTACHMENT_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    match valid {
        true => Ok(()),
        false => Err(format!(
            "a Subagent id is 1 to {MAX_ATTACHMENT_ID} letters, digits, '-', '_', '.' or ':'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(plan: &mut Plan, title: &str, blocked_by: &[TaskId]) -> Result<String, String> {
        plan.apply(PlanOp::Add {
            title: title.into(),
            after: None,
            blocked_by: blocked_by.to_vec(),
            link: None,
        })
    }

    fn status(plan: &mut Plan, task: TaskId, status: TaskStatus) {
        plan.apply(PlanOp::Status { task, status }).unwrap();
    }

    #[test]
    fn tasks_get_fresh_ids_in_order_and_after_inserts_behind_a_task() {
        let mut plan = Plan::default();
        add(&mut plan, "one", &[]).unwrap();
        add(&mut plan, "three", &[]).unwrap();
        plan.apply(PlanOp::Add {
            title: "two".into(),
            after: Some(1),
            blocked_by: vec![],
            link: Some("#241".into()),
        })
        .unwrap();
        let order: Vec<_> = plan
            .tasks
            .iter()
            .map(|t| (t.id, t.title.as_str()))
            .collect();
        assert_eq!(order, [(1, "one"), (3, "two"), (2, "three")]);
        assert_eq!(plan.task(3).unwrap().link.as_deref(), Some("#241"));
    }

    #[test]
    fn a_task_is_ready_once_its_blockers_are_done_or_dropped() {
        let mut plan = Plan::default();
        add(&mut plan, "a", &[]).unwrap();
        add(&mut plan, "b", &[]).unwrap();
        add(&mut plan, "c", &[1, 2]).unwrap();
        let c = |plan: &Plan| plan.is_ready(plan.task(3).unwrap());
        assert!(!c(&plan));
        status(&mut plan, 1, TaskStatus::Done);
        assert!(!c(&plan));
        assert_eq!(plan.open_blockers(plan.task(3).unwrap()), [2]);
        status(&mut plan, 2, TaskStatus::Dropped);
        assert!(c(&plan));
    }

    #[test]
    fn a_subagent_naming_a_task_in_its_description_is_attached_once() {
        let mut plan = Plan::default();
        add(&mut plan, "build", &[]).unwrap();
        assert_eq!(task_marker("Fix it [Task: 1]"), Some(1));
        assert_eq!(task_marker("[task:x] then [task:7]"), Some(7));
        assert_eq!(task_marker("no task"), None);

        assert!(plan.attach_named("sub-1", "[task:1] build it"));
        assert!(!plan.attach_named("sub-1", "[task:1] build it"));
        assert!(!plan.attach_named("sub-2", "[task:9] unknown"));
        assert!(!plan.attach_named("sub-3", "unnamed"));
        assert_eq!(
            plan.task(1).unwrap().attachments,
            [Attachment::Subagent("sub-1".into())]
        );
    }

    #[test]
    fn progress_counts_done_out_of_tasks_not_dropped() {
        let mut plan = Plan::default();
        for title in ["a", "b", "c", "d"] {
            add(&mut plan, title, &[]).unwrap();
        }
        status(&mut plan, 1, TaskStatus::Done);
        status(&mut plan, 2, TaskStatus::Dropped);
        status(&mut plan, 3, TaskStatus::InProgress);
        assert_eq!(plan.progress(), (1, 3));
    }

    #[test]
    fn cycles_and_self_blocks_are_refused_and_leave_the_plan_unchanged() {
        let mut plan = Plan::default();
        add(&mut plan, "a", &[]).unwrap();
        add(&mut plan, "b", &[1]).unwrap();
        add(&mut plan, "c", &[2]).unwrap();
        let before = plan.clone();
        assert!(
            plan.apply(PlanOp::Block {
                task: 1,
                by: vec![3]
            })
            .unwrap_err()
            .contains("cycle")
        );
        assert!(
            plan.apply(PlanOp::Block {
                task: 2,
                by: vec![2]
            })
            .is_err()
        );
        assert_eq!(plan, before);
    }

    #[test]
    fn unknown_ids_and_bad_text_are_refused() {
        let mut plan = Plan::default();
        assert!(add(&mut plan, "a", &[7]).is_err());
        assert!(plan.is_empty(), "a refused add adds nothing");
        assert!(add(&mut plan, "  ", &[]).is_err());
        assert!(add(&mut plan, "bad\u{1b}[2J", &[]).is_err());
        assert!(add(&mut plan, &"x".repeat(MAX_TITLE + 1), &[]).is_err());
        add(&mut plan, "a", &[]).unwrap();
        assert!(
            plan.apply(PlanOp::Status {
                task: 9,
                status: TaskStatus::Done
            })
            .is_err()
        );
        assert!(
            plan.apply(PlanOp::Attach {
                task: 1,
                attachment: Attachment::Subagent("../x y".into()),
            })
            .is_err()
        );
        assert!(
            plan.apply(PlanOp::Link {
                task: 1,
                link: Some("l".repeat(MAX_LINK + 1))
            })
            .is_err()
        );
    }

    #[test]
    fn a_plan_holds_a_bounded_number_of_tasks() {
        let mut plan = Plan::default();
        for _ in 0..MAX_TASKS {
            add(&mut plan, "t", &[]).unwrap();
        }
        assert!(add(&mut plan, "one more", &[]).is_err());
    }

    #[test]
    fn attaching_a_subagent_twice_keeps_one() {
        let mut plan = Plan::default();
        add(&mut plan, "a", &[]).unwrap();
        for _ in 0..2 {
            plan.apply(PlanOp::Attach {
                task: 1,
                attachment: Attachment::Subagent("agent-1".into()),
            })
            .unwrap();
        }
        assert_eq!(plan.task(1).unwrap().attachments.len(), 1);
    }
}
