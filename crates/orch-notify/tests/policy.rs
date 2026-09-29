mod common;

use std::time::{Duration, Instant};

use common::{close_merged, close_session, session, show_merged, show_session};
use orch_config::{ConfigLoader, Notifications, REPO_FILE};
use orch_core::Attention;
use orch_notify::{
    Actions, AttentionContext, AttentionEvent, ClientId, ClientView, NotificationPolicy, Ring,
};

fn event(id: &str, attention: Attention, at: Instant) -> AttentionEvent {
    AttentionEvent {
        session: session(id),
        title: id.to_string(),
        branch: format!("orch/{id}"),
        attention,
        at,
    }
}

fn client(id: u64, focused: bool, showing: Option<&str>) -> ClientView {
    ClientView {
        id: ClientId(id),
        focused,
        showing: showing.map(session),
    }
}

fn repo_notifications(global: &str, repo_file: &str) -> Notifications {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let loader = ConfigLoader::new(dir.path().join("config.toml"));
    std::fs::write(loader.global_path(), global).unwrap();
    std::fs::write(repo.join(REPO_FILE), repo_file).unwrap();
    loader.repo(&repo, None).unwrap().notifications().clone()
}

struct World {
    notifications: Notifications,
    clients: Vec<ClientView>,
    muted: bool,
}

impl World {
    fn new() -> Self {
        Self {
            notifications: Notifications::default(),
            clients: Vec::new(),
            muted: false,
        }
    }

    fn context(&self) -> AttentionContext<'_> {
        AttentionContext {
            muted: self.muted,
            notifications: &self.notifications,
            clients: &self.clients,
        }
    }
}

#[test]
fn a_session_needing_input_shows_a_desktop_notification_and_rings_every_client() {
    let mut world = World::new();
    world.clients = vec![client(1, false, Some("fix-login")), client(2, true, None)];
    let mut policy = NotificationPolicy::new();

    let actions = policy.on_attention(
        event("fix-login", Attention::NeedsInput, Instant::now()),
        world.context(),
    );

    assert_eq!(
        actions,
        Actions {
            desktop: vec![show_session(
                "fix-login",
                "fix-login",
                "Needs input · orch/fix-login"
            )],
            bell: vec![
                Ring {
                    client: ClientId(1),
                    title: "fix-login".into(),
                    body: "Needs input · orch/fix-login".into(),
                },
                Ring {
                    client: ClientId(2),
                    title: "fix-login".into(),
                    body: "Needs input · orch/fix-login".into(),
                },
            ],
        }
    );
}

#[test]
fn nothing_is_sent_for_a_session_visible_in_a_focused_client() {
    let mut world = World::new();
    world.clients = vec![client(1, true, Some("fix-login")), client(2, false, None)];
    let mut policy = NotificationPolicy::new();

    let actions = policy.on_attention(
        event("fix-login", Attention::TurnEnded, Instant::now()),
        world.context(),
    );

    assert_eq!(actions, Actions::default());
}

#[test]
fn a_focused_client_showing_another_session_does_not_suppress() {
    let mut world = World::new();
    world.clients = vec![client(1, true, Some("other"))];
    let mut policy = NotificationPolicy::new();

    let actions = policy.on_attention(
        event("fix-login", Attention::TurnEnded, Instant::now()),
        world.context(),
    );

    assert_eq!(actions.desktop.len(), 1);
    assert_eq!(actions.bell.len(), 1);
}

#[test]
fn a_muted_session_sends_nothing_outside_the_tui() {
    let mut world = World::new();
    world.clients = vec![client(1, false, None)];
    world.muted = true;
    let mut policy = NotificationPolicy::new();

    let actions = policy.on_attention(
        event("fix-login", Attention::Errored, Instant::now()),
        world.context(),
    );

    assert_eq!(actions, Actions::default());
}

#[test]
fn triggers_are_enabled_per_channel() {
    let mut world = World::new();
    world.clients = vec![client(1, false, None)];
    world.notifications = repo_notifications(
        "[notifications.desktop]\nturn_ended = false\n[notifications.bell]\nerrored = false\n",
        "",
    );
    let mut policy = NotificationPolicy::new();
    let now = Instant::now();

    let turn_ended = policy.on_attention(event("a", Attention::TurnEnded, now), world.context());
    let errored = policy.on_attention(
        event("b", Attention::Errored, now + Duration::from_secs(60)),
        world.context(),
    );

    assert!(turn_ended.desktop.is_empty());
    assert_eq!(turn_ended.bell.len(), 1);
    assert_eq!(errored.desktop.len(), 1);
    assert!(errored.bell.is_empty());
}

#[test]
fn a_repo_override_decides_for_sessions_of_that_repo() {
    let global = "[notifications.desktop]\nchecks_failing = false\n";
    let mut quiet = World::new();
    quiet.notifications = repo_notifications(global, "");
    let mut loud = World::new();
    loud.notifications =
        repo_notifications(global, "[notifications.desktop]\nchecks_failing = true\n");
    let mut policy = NotificationPolicy::new();
    let now = Instant::now();

    let from_quiet =
        policy.on_attention(event("a", Attention::ChecksFailing, now), quiet.context());
    let from_loud = policy.on_attention(
        event("b", Attention::ChecksFailing, now + Duration::from_secs(60)),
        loud.context(),
    );

    assert!(from_quiet.desktop.is_empty());
    assert_eq!(
        from_loud.desktop,
        vec![show_session("b", "b", "Checks failing · orch/b")]
    );
}

#[test]
fn a_session_notification_is_replaced_in_place() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let now = Instant::now();

    policy.on_attention(event("a", Attention::NeedsInput, now), world.context());
    let later = policy.on_attention(
        event("a", Attention::TurnEnded, now + Duration::from_secs(1)),
        world.context(),
    );

    assert_eq!(
        later.desktop,
        vec![show_session("a", "a", "Finished its turn · orch/a")]
    );
}

#[test]
fn sessions_changing_within_three_seconds_merge_into_one_notification() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();

    let first = policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    let second = policy.on_attention(
        event("b", Attention::TurnEnded, t0 + Duration::from_secs(1)),
        world.context(),
    );
    let third = policy.on_attention(
        event("c", Attention::Errored, t0 + Duration::from_millis(3500)),
        world.context(),
    );

    assert_eq!(
        first.desktop,
        vec![show_session("a", "a", "Needs input · orch/a")]
    );
    assert_eq!(
        second.desktop,
        vec![
            close_session("a"),
            show_merged(
                "2 Sessions need you",
                "a: Needs input\nb: Finished its turn",
                "b"
            ),
        ]
    );
    assert_eq!(
        third.desktop,
        vec![show_merged(
            "3 Sessions need you",
            "a: Needs input\nb: Finished its turn\nc: Errored",
            "c"
        )]
    );
}

#[test]
fn a_session_changing_again_inside_a_merged_notification_is_listed_once() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();

    policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    policy.on_attention(
        event("b", Attention::NeedsInput, t0 + Duration::from_secs(1)),
        world.context(),
    );
    let again = policy.on_attention(
        event("a", Attention::TurnEnded, t0 + Duration::from_secs(2)),
        world.context(),
    );

    assert_eq!(
        again.desktop,
        vec![show_merged(
            "2 Sessions need you",
            "a: Finished its turn\nb: Needs input",
            "a"
        )]
    );
}

#[test]
fn after_a_quiet_window_sessions_are_notified_individually_again() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();

    policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    policy.on_attention(
        event("b", Attention::NeedsInput, t0 + Duration::from_secs(1)),
        world.context(),
    );
    let quiet = policy.on_attention(
        event("c", Attention::PrMerged, t0 + Duration::from_secs(5)),
        world.context(),
    );

    assert_eq!(
        quiet.desktop,
        vec![show_session("c", "c", "PR merged · orch/c")]
    );
}

#[test]
fn dismissing_a_session_closes_its_notification_once() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();

    policy.on_attention(
        event("a", Attention::NeedsInput, Instant::now()),
        world.context(),
    );

    assert_eq!(policy.dismiss(&session("a")), vec![close_session("a")]);
    assert_eq!(policy.dismiss(&session("a")), vec![]);
}

#[test]
fn dismissing_down_to_one_session_turns_the_merged_notification_back_into_its_own() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();
    policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    policy.on_attention(
        event("b", Attention::Errored, t0 + Duration::from_secs(1)),
        world.context(),
    );
    policy.on_attention(
        event("c", Attention::Errored, t0 + Duration::from_secs(2)),
        world.context(),
    );

    assert_eq!(
        policy.dismiss(&session("c")),
        vec![show_merged(
            "2 Sessions need you",
            "a: Needs input\nb: Errored",
            "b"
        )]
    );
    assert_eq!(
        policy.dismiss(&session("b")),
        vec![
            close_merged(),
            show_session("a", "a", "Needs input · orch/a")
        ]
    );
    assert_eq!(policy.dismiss(&session("a")), vec![close_session("a")]);
}

#[test]
fn a_session_already_merged_updates_the_merged_notification_even_after_the_window() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();
    policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    policy.on_attention(
        event("b", Attention::NeedsInput, t0 + Duration::from_secs(1)),
        world.context(),
    );

    let later = policy.on_attention(
        event("b", Attention::TurnEnded, t0 + Duration::from_secs(30)),
        world.context(),
    );

    assert_eq!(
        later.desktop,
        vec![show_merged(
            "2 Sessions need you",
            "a: Needs input\nb: Finished its turn",
            "b"
        )]
    );
}

#[test]
fn pr_events_notify_with_their_own_wording() {
    let world = World::new();
    let t0 = Instant::now();
    let cases = [
        (Attention::ChecksFailing, "Checks failing · orch/pr"),
        (Attention::ChangesRequested, "Changes requested · orch/pr"),
        (Attention::PrMerged, "PR merged · orch/pr"),
    ];
    for (attention, body) in cases {
        let mut policy = NotificationPolicy::new();
        let actions = policy.on_attention(event("pr", attention, t0), world.context());
        assert_eq!(
            actions.desktop,
            vec![show_session("pr", "pr", body)],
            "{attention:?}"
        );
    }
}

#[test]
fn a_closed_pr_only_notifies_when_enabled() {
    let mut world = World::new();
    let t0 = Instant::now();

    let by_default = NotificationPolicy::new()
        .on_attention(event("pr", Attention::PrClosed, t0), world.context());
    world.notifications = repo_notifications("[notifications.desktop]\npr_closed = true\n", "");
    let enabled = NotificationPolicy::new()
        .on_attention(event("pr", Attention::PrClosed, t0), world.context());

    assert_eq!(by_default, Actions::default());
    assert_eq!(
        enabled.desktop,
        vec![show_session("pr", "pr", "PR closed · orch/pr")]
    );
}

#[test]
fn a_dismissed_session_does_not_merge_with_the_next_one() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();

    policy.on_attention(event("a", Attention::NeedsInput, t0), world.context());
    policy.dismiss(&session("a"));
    let next = policy.on_attention(
        event("b", Attention::TurnEnded, t0 + Duration::from_secs(1)),
        world.context(),
    );

    assert_eq!(
        next.desktop,
        vec![show_session("b", "b", "Finished its turn · orch/b")]
    );
}

#[test]
fn a_steady_stream_keeps_merging_because_the_window_slides() {
    let world = World::new();
    let mut policy = NotificationPolicy::new();
    let t0 = Instant::now();

    for (n, id) in ["a", "b", "c"].into_iter().enumerate() {
        let at = t0 + Duration::from_secs(2 * n as u64);
        policy.on_attention(event(id, Attention::NeedsInput, at), world.context());
    }
    let fourth = policy.on_attention(
        event("d", Attention::NeedsInput, t0 + Duration::from_secs(6)),
        world.context(),
    );

    assert_eq!(
        fourth.desktop,
        vec![show_merged(
            "4 Sessions need you",
            "a: Needs input\nb: Needs input\nc: Needs input\nd: Needs input",
            "d"
        )]
    );
}
