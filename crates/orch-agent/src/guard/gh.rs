use super::shell::Word;
use super::{GuardHit, other_ref};

const MUTATING_METHODS: [&str; 4] = ["POST", "PUT", "PATCH", "DELETE"];
const FIELD_OPTIONS: [&str; 5] = ["-f", "-F", "--field", "--raw-field", "--input"];

pub(super) fn hits(args: &[&Word]) -> Option<GuardHit> {
    let words: Vec<&str> = args.iter().map(|word| word.text.as_str()).collect();
    match words.as_slice() {
        ["pr", "merge", rest @ ..] => {
            let number = rest.iter().find(|word| !word.starts_with('-'));
            Some(other_ref(&match number {
                Some(number) => format!("gh pr merge {number}"),
                None => "gh pr merge".into(),
            }))
        }
        ["api", rest @ ..] => api(rest),
        _ => None,
    }
}

fn api(args: &[&str]) -> Option<GuardHit> {
    let mut method = None;
    let mut sends_fields = false;
    let mut endpoint = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(value) = arg
            .strip_prefix("--method=")
            .or_else(|| arg.strip_prefix("-X").filter(|value| !value.is_empty()))
        {
            method = Some(value.to_string());
        } else if matches!(*arg, "-X" | "--method") {
            method = args.next().map(|value| value.to_string());
        } else if FIELD_OPTIONS.contains(arg) {
            sends_fields = true;
            args.next();
        } else if arg.starts_with('-') {
            continue;
        } else if endpoint.is_none() {
            endpoint = Some(*arg);
        }
    }
    let method = method.map(|method| method.to_ascii_uppercase());
    let mutating = match method.as_deref() {
        Some(method) => MUTATING_METHODS.contains(&method),
        None => sends_fields,
    };
    mutating.then(|| other_ref(&format!("gh api {}", endpoint.unwrap_or_default())))
}
