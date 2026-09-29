use orch_protocol::Request;

use crate::app::{App, Call};
use crate::event::{Effect, ReviewPurpose};

struct Command {
    names: &'static [&'static str],
    run: fn(&mut App, &str),
}

const COMMANDS: &[Command] = &[
    Command {
        names: &["q", "quit", "qa"],
        run: |app, _| app.push(Call::Local(Effect::Quit)),
    },
    Command {
        names: &["new"],
        run: |app, _| app.open_new_form(),
    },
    Command {
        names: &["land"],
        run: |app, _| app.open_land(),
    },
    Command {
        names: &["abandon"],
        run: |app, _| app.on_selected(|session| Request::AbandonPr { session }),
    },
    Command {
        names: &["discard"],
        run: |app, _| app.load_discard_preview(),
    },
    Command {
        names: &["review"],
        run: |app, _| app.load_review(ReviewPurpose::BuiltIn),
    },
    Command {
        names: &["review!", "Review"],
        run: |app, _| app.load_review(ReviewPurpose::External),
    },
    Command {
        names: &["resume"],
        run: |app, _| app.resume_selected(false),
    },
    Command {
        names: &["retry"],
        run: |app, _| app.on_selected(|session| Request::RetrySetup { session }),
    },
    Command {
        names: &["start"],
        run: |app, _| app.on_selected(|session| Request::StartAnyway { session }),
    },
    Command {
        names: &["guard"],
        run: |app, _| app.reveal_guards(),
    },
    Command {
        names: &["guards"],
        run: guards,
    },
    Command {
        names: &["preset"],
        run: preset,
    },
    Command {
        names: &["mute"],
        run: mute,
    },
    Command {
        names: &["usage"],
        run: |app, _| not_available(app, "usage"),
    },
    Command {
        names: &["reconcile"],
        run: |app, _| not_available(app, "reconcile"),
    },
];

pub(crate) fn run(app: &mut App, line: &str) {
    let line = line.trim();
    let (name, args) = line.split_once(' ').unwrap_or((line, ""));
    if name.is_empty() {
        return;
    }
    match COMMANDS
        .iter()
        .find(|command| command.names.contains(&name))
    {
        Some(command) => (command.run)(app, args.trim()),
        None => app.message = Some(format!("unknown command: {name}")),
    }
}

fn guards(app: &mut App, args: &str) {
    let enabled = match args {
        "on" => true,
        "off" => false,
        _ => {
            app.message = Some("usage: :guards off|on".into());
            return;
        }
    };
    app.on_selected(|session| Request::SetGuards { session, enabled });
}

fn preset(app: &mut App, args: &str) {
    if args.is_empty() {
        app.message = Some("usage: :preset <name>".into());
        return;
    }
    let preset = args.to_string();
    app.on_selected(|session| Request::SetPreset { session, preset });
}

fn mute(app: &mut App, _: &str) {
    let muted = !app.selected_view().is_some_and(|view| view.flags.muted);
    app.on_selected(|session| Request::SetMuted { session, muted });
}

fn not_available(app: &mut App, name: &str) {
    app.message = Some(format!(
        ":{name} is not available yet (the Daemon does not support it)"
    ));
}
