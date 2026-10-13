use std::path::Path;

use super::shell::Word;
use super::{GuardHit, GuardKind, GuardScope, other_ref, paths};

const FILTERS: [&str; 7] = [
    "--contains",
    "--no-contains",
    "--merged",
    "--no-merged",
    "--points-at",
    "--format",
    "--sort",
];

const CHECKOUT_MUTATIONS: [&str; 18] = [
    "add",
    "am",
    "apply",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "merge",
    "mv",
    "pull",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "switch",
    "worktree",
];

#[derive(Default)]
struct Args<'w> {
    flags: Vec<&'w str>,
    values: Vec<(&'w str, &'w Word)>,
    operands: Vec<&'w Word>,
    paths_follow: bool,
}

impl<'w> Args<'w> {
    fn parse(words: &[&'w Word], takes_value: &[&str]) -> Self {
        let mut args = Args::default();
        let mut words = words.iter().copied();
        while let Some(word) = words.next() {
            let text = word.text.as_str();
            if text == "--" {
                args.paths_follow = true;
                break;
            } else if takes_value.contains(&text) {
                if let Some(value) = words.next() {
                    args.values.push((text, value));
                }
            } else if text.starts_with('-') && text.len() > 1 {
                args.flags.push(text);
            } else {
                args.operands.push(word);
            }
        }
        args
    }

    fn has(&self, flags: &[&str]) -> bool {
        self.flags.iter().any(|flag| {
            flags
                .iter()
                .any(|wanted| flag == wanted || flag.starts_with(&format!("{wanted}=")))
        })
    }

    fn value(&self, options: &[&str]) -> Option<&'w Word> {
        self.values
            .iter()
            .find(|(option, _)| options.contains(option))
            .map(|(_, value)| *value)
    }

    fn named(&self, all: bool) -> impl Iterator<Item = &'w str> + '_ {
        self.known_operands().take(if all { usize::MAX } else { 1 })
    }

    fn known_operands(&self) -> impl Iterator<Item = &'w str> + '_ {
        self.operands
            .iter()
            .filter(|word| !word.dynamic)
            .map(|word| word.text.as_str())
    }
}

pub(super) fn hits(scope: &GuardScope, cwd: &Path, args: &[&Word]) -> Vec<GuardHit> {
    let mut git_cwd = cwd.to_path_buf();
    let mut words = args.iter().copied();
    let mut subcommand = None;
    while let Some(word) = words.next() {
        match word.text.as_str() {
            "-C" => {
                if let Some(dir) = words.next().filter(|dir| !dir.dynamic) {
                    git_cwd = paths::resolve(&git_cwd, &dir.text);
                }
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                words.next();
            }
            option if option.starts_with('-') => {}
            _ => {
                subcommand = Some(word.text.as_str());
                break;
            }
        }
    }
    let Some(subcommand) = subcommand else {
        return Vec::new();
    };
    let rest: Vec<&Word> = words.collect();

    let mut hits: Vec<GuardHit> = match subcommand {
        "worktree" => worktree(&rest).into_iter().collect(),
        "push" => push(scope, &rest),
        "fetch" | "pull" => fetch(scope, &rest),
        "branch" => branch(scope, &rest),
        "tag" => tag(&rest),
        "checkout" => checkout(scope, &git_cwd, &rest).into_iter().collect(),
        "switch" => switch(scope, &rest).into_iter().collect(),
        "update-ref" => update_ref(scope, &rest),
        "symbolic-ref" => symbolic_ref(scope, &rest),
        _ => Vec::new(),
    };
    if CHECKOUT_MUTATIONS.contains(&subcommand) && scope.outside(&git_cwd) {
        hits.push(GuardHit {
            kind: GuardKind::WriteOutsideWorktree,
            target: git_cwd.to_string_lossy().into_owned(),
        });
    }
    hits
}

fn worktree(rest: &[&Word]) -> Option<GuardHit> {
    let action = rest.iter().find(|word| !word.text.starts_with('-'))?;
    (action.text != "list").then(|| GuardHit {
        kind: GuardKind::WorktreeManagement,
        target: action.text.clone(),
    })
}

fn push(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(
        rest,
        &["-o", "--push-option", "--repo", "--receive-pack", "--exec"],
    );
    if args.has(&["-n", "--dry-run"]) {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (flags, target) in [
        (&["--all", "--branches"][..], "all branches"),
        (&["--mirror"][..], "all refs"),
        (&["--tags"][..], "tags"),
    ] {
        if args.has(flags) {
            hits.push(other_ref(target));
        }
    }
    let refspecs = args.known_operands().skip(1);
    hits.extend(refspecs.filter_map(|refspec| {
        let refspec = refspec.trim_start_matches('+');
        let destination = match refspec.split_once(':') {
            Some((_, destination)) => destination,
            None => refspec,
        };
        scope.ref_hit(destination)
    }));
    hits
}

fn fetch(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(
        rest,
        &[
            "--depth",
            "--deepen",
            "-j",
            "--jobs",
            "--shallow-since",
            "--shallow-exclude",
            "--upload-pack",
            "-o",
            "--server-option",
            "--negotiation-tip",
            "-s",
            "--strategy",
            "-X",
            "--strategy-option",
        ],
    );
    args.known_operands()
        .skip(1)
        .filter_map(|refspec| refspec.split_once(':'))
        .map(|(_, destination)| destination.trim_start_matches('+'))
        .filter(|destination| !destination.is_empty())
        .filter_map(|destination| scope.ref_hit(destination))
        .collect()
}

fn branch(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(rest, &[&["-u", "--set-upstream-to"][..], &FILTERS].concat());
    let renames_or_deletes = args.has(&[
        "-d", "-D", "--delete", "-m", "-M", "--move", "-c", "-C", "--copy",
    ]);
    let lists = args.has(&[
        "-l",
        "--list",
        "-a",
        "--all",
        "-r",
        "--remotes",
        "-v",
        "-vv",
        "--verbose",
        "--show-current",
    ]) || args.value(&FILTERS).is_some();
    if lists && !renames_or_deletes {
        return Vec::new();
    }
    args.named(renames_or_deletes)
        .filter_map(|name| scope.ref_hit(name))
        .collect()
}

fn tag(rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(
        rest,
        &[
            &[
                "-m",
                "--message",
                "-F",
                "--file",
                "-u",
                "--local-user",
                "--cleanup",
            ][..],
            &FILTERS,
        ]
        .concat(),
    );
    let deletes = args.has(&["-d", "--delete"]);
    let lists = args.has(&["-l", "--list", "-v", "--verify"])
        || args.flags.iter().any(|flag| flag.starts_with("-n"))
        || args.value(&FILTERS).is_some();
    if lists && !deletes {
        return Vec::new();
    }
    args.named(deletes).map(other_ref).collect()
}

fn checkout(scope: &GuardScope, cwd: &Path, rest: &[&Word]) -> Option<GuardHit> {
    let args = Args::parse(rest, &["-b", "-B", "--orphan"]);
    if let Some(new_branch) = args.value(&["-b", "-B", "--orphan"]) {
        return (!new_branch.dynamic)
            .then(|| scope.ref_hit(&new_branch.text))
            .flatten();
    }
    if args.paths_follow || args.operands.len() != 1 {
        return None;
    }
    let target = args.known_operands().next()?;
    let hit = scope.ref_hit(target)?;
    let looks_like_path = target.starts_with(['.', '/']) || cwd.join(target).exists();
    (hit.kind == GuardKind::BaseBranch || !looks_like_path).then_some(hit)
}

fn switch(scope: &GuardScope, rest: &[&Word]) -> Option<GuardHit> {
    let creating = ["-c", "-C", "--create", "--force-create", "--orphan"];
    let args = Args::parse(rest, &creating);
    let target = match args.value(&creating) {
        Some(new_branch) => (!new_branch.dynamic).then_some(new_branch.text.as_str())?,
        None => args.known_operands().next()?,
    };
    scope.ref_hit(target)
}

fn update_ref(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(rest, &["-m"]);
    if args.has(&["--stdin"]) {
        return vec![other_ref("stdin")];
    }
    args.known_operands()
        .next()
        .and_then(|name| scope.ref_hit(name))
        .into_iter()
        .collect()
}

fn symbolic_ref(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(rest, &["-m"]);
    let operands: Vec<&str> = args.known_operands().collect();
    let target = if args.has(&["-d", "--delete"]) {
        operands.first()
    } else {
        operands.get(1)
    };
    target
        .and_then(|name| scope.ref_hit(name))
        .into_iter()
        .collect()
}
