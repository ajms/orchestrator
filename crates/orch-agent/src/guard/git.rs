use std::path::{Path, PathBuf};

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

    fn named(&self, all: bool) -> Vec<String> {
        self.known(0, if all { usize::MAX } else { 1 })
    }

    fn known(&self, skip: usize, take: usize) -> Vec<String> {
        self.operands
            .iter()
            .skip(skip)
            .take(take)
            .flat_map(|word| known(word))
            .collect()
    }
}

fn known(word: &Word) -> Vec<String> {
    if word.expanded {
        return Vec::new();
    }
    word.candidates().unwrap_or_else(|| vec![word.text.clone()])
}

fn first_hit(scope: &GuardScope, names: Vec<String>) -> Option<GuardHit> {
    names.iter().find_map(|name| scope.ref_hit(name))
}

fn directory(scope: &GuardScope, cwd: &Path, dir: &Word) -> Option<PathBuf> {
    if dir.expanded {
        return None;
    }
    let Some(candidates) = dir.candidates() else {
        return Some(PathBuf::from(&dir.text));
    };
    let resolved: Vec<PathBuf> = candidates
        .iter()
        .map(|candidate| paths::resolve(cwd, &paths::widest(candidate)))
        .collect();
    resolved
        .iter()
        .find(|path| scope.outside(path))
        .or(resolved.first())
        .cloned()
}

pub(super) fn hits(scope: &GuardScope, cwd: &Path, args: &[&Word]) -> Vec<GuardHit> {
    let mut git_cwd = cwd.to_path_buf();
    let mut words = args.iter().copied();
    let mut subcommand = None;
    while let Some(word) = words.next() {
        match word.text.as_str() {
            "-C" => {
                if let Some(dir) = words.next().and_then(|dir| directory(scope, &git_cwd, dir)) {
                    git_cwd = dir;
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
    let refspecs = args.known(1, usize::MAX);
    hits.extend(refspecs.iter().filter_map(|refspec| {
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
    args.known(1, usize::MAX)
        .iter()
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
        .iter()
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
    args.named(deletes)
        .iter()
        .map(|name| other_ref(name))
        .collect()
}

fn checkout(scope: &GuardScope, cwd: &Path, rest: &[&Word]) -> Option<GuardHit> {
    let args = Args::parse(rest, &["-b", "-B", "--orphan"]);
    if let Some(new_branch) = args.value(&["-b", "-B", "--orphan"]) {
        return first_hit(scope, known(new_branch));
    }
    if args.paths_follow || args.operands.len() != 1 {
        return None;
    }
    args.named(false).iter().find_map(|target| {
        let hit = scope.ref_hit(target)?;
        let looks_like_path = target.starts_with(['.', '/']) || cwd.join(target).exists();
        (hit.kind == GuardKind::BaseBranch || !looks_like_path).then_some(hit)
    })
}

fn switch(scope: &GuardScope, rest: &[&Word]) -> Option<GuardHit> {
    let creating = ["-c", "-C", "--create", "--force-create", "--orphan"];
    let args = Args::parse(rest, &creating);
    let targets = match args.value(&creating) {
        Some(new_branch) => known(new_branch),
        None => args.named(false),
    };
    first_hit(scope, targets)
}

fn update_ref(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(rest, &["-m"]);
    if args.has(&["--stdin"]) {
        return vec![other_ref("stdin")];
    }
    first_hit(scope, args.named(false)).into_iter().collect()
}

fn symbolic_ref(scope: &GuardScope, rest: &[&Word]) -> Vec<GuardHit> {
    let args = Args::parse(rest, &["-m"]);
    let target = if args.has(&["-d", "--delete"]) { 0 } else { 1 };
    first_hit(scope, args.known(target, 1))
        .into_iter()
        .collect()
}
