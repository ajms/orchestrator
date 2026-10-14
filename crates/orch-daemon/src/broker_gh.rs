#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitHubRepo {
    owner: String,
    name: String,
}

struct Named<'a> {
    host: Option<&'a str>,
    owner: &'a str,
    name: &'a str,
    rest: &'a str,
}

impl<'a> Named<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let text = text.trim().trim_end_matches('/');
        let (host, path) = match text.split_once("://") {
            Some((_, rest)) => rest
                .split_once('/')
                .map(|(host, path)| (Some(host), path))?,
            None => match text.split_once(':') {
                Some((host, path)) if !host.contains('/') => (Some(host), path),
                _ => (None, text),
            },
        };
        let host = host.and_then(|host| host.rsplit('@').next()?.split(':').next());
        let mut parts = path.trim_start_matches('/').splitn(3, '/');
        let first = parts.next()?;
        let (host, owner) = match host {
            None if first.contains('.') => (Some(first), parts.next()?),
            host => (host, first),
        };
        let name = parts.next()?;
        Some(Named {
            host: host.map(|host| host.trim_start_matches("www.")),
            owner,
            name: name.strip_suffix(".git").unwrap_or(name),
            rest: parts.next().unwrap_or_default(),
        })
    }

    fn on_github(&self) -> bool {
        self.host.is_none_or(|host| {
            ["github.com", "ssh.github.com"]
                .iter()
                .any(|github| host.eq_ignore_ascii_case(github))
        })
    }

    fn names(&self, repo: &GitHubRepo) -> bool {
        self.on_github()
            && self.owner.eq_ignore_ascii_case(&repo.owner)
            && self.name.eq_ignore_ascii_case(&repo.name)
    }
}

impl GitHubRepo {
    pub(crate) fn from_remote(url: &str) -> Option<Self> {
        let named = Named::parse(url)?;
        let host = named.host?;
        if host.contains('.') && !named.on_github() || !named.rest.is_empty() {
            return None;
        }
        let valid = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        (valid(named.owner) && valid(named.name)).then(|| GitHubRepo {
            owner: named.owner.into(),
            name: named.name.into(),
        })
    }

    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    fn is(&self, named: &str) -> bool {
        Named::parse(named).is_some_and(|named| named.rest.is_empty() && named.names(self))
    }

    fn owns_url(&self, url: &str) -> bool {
        Named::parse(url).is_some_and(|named| named.host.is_some() && named.names(self))
    }
}

pub(crate) struct Limits<'a> {
    pub(crate) repo: &'a GitHubRepo,
    pub(crate) branch: &'a str,
    pub(crate) base: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PrChange {
    Create,
    Edit { pr: Option<u64> },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GhCall {
    pub(crate) args: Vec<String>,
    pub(crate) said: String,
    pub(crate) change: Option<PrChange>,
}

struct Flag {
    long: &'static str,
    short: Option<char>,
    value: bool,
}

const fn value(long: &'static str, short: Option<char>) -> Flag {
    Flag {
        long,
        short,
        value: true,
    }
}

const fn switch(long: &'static str, short: Option<char>) -> Flag {
    Flag {
        long,
        short,
        value: false,
    }
}

const REPO: Flag = value("repo", Some('R'));
const OUTPUT: &[Flag] = &[
    REPO,
    value("json", None),
    value("jq", Some('q')),
    value("template", Some('t')),
];
const ISSUE_VIEW: &[Flag] = &[switch("comments", Some('c'))];
const ISSUE_LIST: &[Flag] = &[
    value("state", Some('s')),
    value("label", Some('l')),
    value("limit", Some('L')),
    value("assignee", Some('a')),
    value("author", Some('A')),
    value("search", Some('S')),
    value("milestone", Some('m')),
    value("mention", None),
    value("app", None),
];
const PR_LIST: &[Flag] = &[
    value("state", Some('s')),
    value("label", Some('l')),
    value("limit", Some('L')),
    value("base", Some('B')),
    value("head", Some('H')),
    value("author", Some('A')),
    value("assignee", Some('a')),
    value("search", Some('S')),
    value("app", None),
    switch("draft", Some('d')),
];
const PR_DIFF: &[Flag] = &[
    REPO,
    switch("patch", None),
    switch("name-only", None),
    value("color", None),
];
const PR_CHECKS: &[Flag] = &[switch("required", None)];
const RUN_VIEW: &[Flag] = &[
    switch("log", None),
    switch("log-failed", None),
    switch("verbose", Some('v')),
    switch("exit-status", None),
    value("job", Some('j')),
    value("attempt", Some('a')),
];
const RUN_LIST: &[Flag] = &[
    value("limit", Some('L')),
    value("branch", Some('b')),
    value("workflow", Some('w')),
    value("status", Some('s')),
    value("event", Some('e')),
    value("user", Some('u')),
    value("commit", Some('c')),
    value("created", None),
    switch("all", Some('a')),
];
const API: &[Flag] = &[
    value("method", Some('X')),
    value("jq", Some('q')),
    value("template", Some('t')),
    value("cache", None),
    switch("paginate", None),
    switch("slurp", None),
    switch("include", Some('i')),
    switch("silent", None),
    switch("verbose", None),
];
const PR_CREATE: &[Flag] = &[
    REPO,
    value("title", Some('t')),
    value("body", Some('b')),
    value("base", Some('B')),
    value("head", Some('H')),
    switch("draft", Some('d')),
    switch("fill", Some('f')),
    switch("fill-first", None),
    switch("fill-verbose", None),
];
const PR_EDIT: &[Flag] = &[
    REPO,
    value("title", Some('t')),
    value("body", Some('b')),
    value("base", Some('B')),
];
const PR_READY: &[Flag] = &[REPO, switch("undo", None)];

struct Parsed {
    flags: Vec<(&'static str, Option<String>)>,
    positionals: Vec<String>,
}

impl Parsed {
    fn take(&mut self, long: &str) -> Vec<String> {
        let (taken, kept) = std::mem::take(&mut self.flags)
            .into_iter()
            .partition(|(name, _)| *name == long);
        self.flags = kept;
        taken.into_iter().filter_map(|(_, value)| value).collect()
    }

    fn args(self) -> Vec<String> {
        let mut args = self.positionals;
        for (name, value) in self.flags {
            args.push(format!("--{name}"));
            args.extend(value);
        }
        args
    }
}

fn parse(args: &[String], tables: &[&[Flag]], what: &str) -> Result<Parsed, String> {
    let find = |matches: &dyn Fn(&Flag) -> bool| {
        tables
            .iter()
            .flat_map(|table| table.iter())
            .find(|flag| matches(flag))
    };
    let mut parsed = Parsed {
        flags: Vec::new(),
        positionals: Vec::new(),
    };
    let mut rest = args.iter();
    let mut flags_over = false;
    while let Some(arg) = rest.next() {
        if flags_over || !arg.starts_with('-') || arg == "-" {
            if arg.starts_with('-') {
                return Err(format!("the Broker doesn't pass {arg} to {what}"));
            }
            parsed.positionals.push(arg.clone());
            continue;
        }
        if arg == "--" {
            flags_over = true;
            continue;
        }
        let (named, flag, inline) = match arg.strip_prefix("--") {
            Some(long) => {
                let (name, inline) = match long.split_once('=') {
                    Some((name, inline)) => (name, Some(inline)),
                    None => (long, None),
                };
                (format!("--{name}"), find(&|flag| flag.long == name), inline)
            }
            None => {
                let mut chars = arg[1..].chars();
                let short = chars.next();
                let tail = chars.as_str();
                let named = format!("-{}", short.unwrap_or_default());
                let flag = find(&|flag| flag.short.is_some() && flag.short == short);
                (named, flag, (!tail.is_empty()).then_some(tail))
            }
        };
        let flag = flag.ok_or_else(|| format!("the Broker doesn't pass {named} to {what}"))?;
        let value = match (flag.value, inline) {
            (true, Some(inline)) => Some(inline.to_string()),
            (true, None) => Some(
                rest.next()
                    .ok_or_else(|| format!("{named} needs a value"))?
                    .clone(),
            ),
            (false, None) => None,
            (false, Some(_)) => return Err(format!("{named} takes no value")),
        };
        parsed.flags.push((flag.long, value));
    }
    Ok(parsed)
}

fn no_env_in_jq(parsed: &Parsed) -> Result<(), String> {
    let reads_env = |query: &str| {
        query.contains("$ENV")
            || query
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|word| word == "env")
    };
    match parsed
        .flags
        .iter()
        .any(|(name, value)| *name == "jq" && value.as_deref().is_some_and(reads_env))
    {
        true => Err("the Broker doesn't pass jq queries that read the environment".into()),
        false => Ok(()),
    }
}

pub(crate) fn gh_call(args: &[String], limits: &Limits) -> Result<GhCall, String> {
    let (command, sub) = match args {
        [command, sub, ..] => (command.as_str(), sub.as_str()),
        [command] => (command.as_str(), ""),
        [] => return Err(refused("gh")),
    };
    let what = format!("gh {command} {sub}").trim_end().to_string();
    if command == "api" {
        return api(&args[1..], limits.repo);
    }
    let (tables, at_most): (&[&[Flag]], usize) = match (command, sub) {
        ("issue", "view") => (&[OUTPUT, ISSUE_VIEW], 1),
        ("issue", "list") => (&[OUTPUT, ISSUE_LIST], 0),
        ("pr", "view") => (&[OUTPUT, ISSUE_VIEW], 1),
        ("pr", "list") => (&[OUTPUT, PR_LIST], 0),
        ("pr", "diff") => (&[PR_DIFF], 1),
        ("pr", "checks") => (&[OUTPUT, PR_CHECKS], 1),
        ("run", "view") => (&[OUTPUT, RUN_VIEW], 1),
        ("run", "list") => (&[OUTPUT, RUN_LIST], 0),
        ("pr", "create") => (&[PR_CREATE], 0),
        ("pr", "edit") => (&[PR_EDIT], 1),
        ("pr", "ready") => (&[PR_READY], 1),
        _ => return Err(refused(&what)),
    };
    let mut parsed = parse(&args[2..], tables, &what)?;
    if parsed.positionals.len() > at_most {
        return Err(format!("too many arguments for {what}"));
    }
    if let Some(other) = parsed
        .take("repo")
        .into_iter()
        .find(|named| !limits.repo.is(named))
    {
        return Err(another_repo(&other, limits.repo));
    }
    if let Some(url) = parsed
        .positionals
        .iter()
        .find(|arg| arg.contains("://") || arg.starts_with("github.com/"))
        && !limits.repo.owns_url(url)
    {
        return Err(another_repo(url, limits.repo));
    }
    no_env_in_jq(&parsed)?;
    let said = match parsed.positionals.first() {
        Some(target) => format!("{what} {target}"),
        None => what.clone(),
    };
    let (mut args, change) = match sub {
        "create" if command == "pr" => {
            pr_create(&mut parsed, limits)?;
            let fixed = [
                "pr",
                "create",
                "--head",
                limits.branch,
                "--base",
                limits.base,
            ];
            (strings(&fixed), Some(PrChange::Create))
        }
        "edit" | "ready" if command == "pr" => {
            let pr = own_pr(parsed.positionals.pop(), limits)?;
            if parsed.take("base").iter().any(|base| base != limits.base) {
                return Err(fixed_base(limits.base));
            }
            (
                strings(&["pr", sub, limits.branch]),
                Some(PrChange::Edit { pr }),
            )
        }
        _ => (strings(&[command, sub]), None),
    };
    args.extend(parsed.args());
    args.extend(strings(&["--repo", &limits.repo.slug()]));
    Ok(GhCall { args, said, change })
}

fn pr_create(parsed: &mut Parsed, limits: &Limits) -> Result<(), String> {
    if parsed.take("head").iter().any(|head| head != limits.branch) {
        return Err(format!(
            "the Broker opens a PR only for this Session's Branch {}",
            limits.branch
        ));
    }
    if parsed.take("base").iter().any(|base| base != limits.base) {
        return Err(fixed_base(limits.base));
    }
    Ok(())
}

fn own_pr(target: Option<String>, limits: &Limits) -> Result<Option<u64>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    if target == limits.branch {
        return Ok(None);
    }
    let number = match target.strip_prefix('#').unwrap_or(&target).parse() {
        Ok(number) => Some(number),
        Err(_) if limits.repo.owns_url(&target) => target
            .trim_end_matches('/')
            .split_once("/pull/")
            .and_then(|(_, number)| number.parse().ok()),
        Err(_) => None,
    };
    number
        .map(Some)
        .ok_or_else(|| not_this_sessions_pr(limits.branch))
}

pub(crate) fn not_this_sessions_pr(branch: &str) -> String {
    format!("the Broker edits only this Session's PR, for {branch}")
}

fn api(args: &[String], repo: &GitHubRepo) -> Result<GhCall, String> {
    let mut parsed = parse(args, &[API], "gh api")?;
    let [endpoint] = parsed.positionals.as_slice() else {
        return Err("gh api takes one endpoint".into());
    };
    let endpoint = endpoint
        .trim_start_matches('/')
        .replace("{owner}", &repo.owner)
        .replace("{repo}", &repo.name);
    let path = endpoint.split('?').next().unwrap_or_default();
    let prefix = format!("repos/{}", repo.slug());
    let under = path.len() >= prefix.len()
        && path[..prefix.len()].eq_ignore_ascii_case(&prefix)
        && matches!(path.as_bytes().get(prefix.len()), None | Some(b'/'));
    if !under || path.contains("..") || path.contains('%') || path.contains('\\') {
        return Err(format!(
            "the Broker's gh api reaches only repos/{} on GitHub",
            repo.slug()
        ));
    }
    if parsed
        .take("method")
        .iter()
        .any(|method| !method.eq_ignore_ascii_case("GET"))
    {
        return Err("the Broker passes only GET requests to gh api".into());
    }
    no_env_in_jq(&parsed)?;
    let said = format!("gh api {endpoint}");
    parsed.positionals.clear();
    let mut args = strings(&["api", &endpoint, "--method", "GET"]);
    args.extend(parsed.args());
    Ok(GhCall {
        args,
        said,
        change: None,
    })
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

fn refused(what: &str) -> String {
    format!(
        "the Broker doesn't run `{what}`: it passes reads of this Repo, and creating or editing this Session's PR"
    )
}

fn another_repo(named: &str, repo: &GitHubRepo) -> String {
    format!(
        "the Broker reaches only this Repo, {}, not {named}",
        repo.slug()
    )
}

fn fixed_base(base: &str) -> String {
    format!("the PR's base is fixed to the Base branch {base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> GitHubRepo {
        GitHubRepo::from_remote("git@github.com:acme/app.git").unwrap()
    }

    fn call(line: &str) -> Result<GhCall, String> {
        let repo = acme();
        let limits = Limits {
            repo: &repo,
            branch: "orch/work",
            base: "main",
        };
        let args: Vec<String> = line.split(' ').map(String::from).collect();
        gh_call(&args, &limits)
    }

    fn passed(line: &str) -> String {
        call(line).unwrap().args.join(" ")
    }

    fn refusal(line: &str) -> String {
        call(line).unwrap_err()
    }

    #[test]
    fn origin_names_the_github_repo_in_any_url_form_but_not_another_host() {
        for url in [
            "git@github.com:acme/app.git",
            "https://github.com/acme/app",
            "ssh://git@github.com/acme/app.git",
            "github-work:acme/app.git",
        ] {
            assert_eq!(GitHubRepo::from_remote(url), Some(acme()), "{url}");
        }
        for url in ["git@gitlab.com:acme/app.git", "/srv/git/app.git"] {
            assert_eq!(GitHubRepo::from_remote(url), None, "{url}");
        }
    }

    #[test]
    fn reads_pass_with_normalised_flags_and_this_repo_pinned() {
        assert_eq!(
            passed("pr view 3 -q.title --json=title"),
            "pr view 3 --jq .title --json title --repo acme/app"
        );
        assert_eq!(
            passed("run list -L5 -R ACME/app"),
            "run list --limit 5 --repo acme/app"
        );
        assert_eq!(
            passed("issue view https://github.com/acme/app/issues/1"),
            "issue view https://github.com/acme/app/issues/1 --repo acme/app"
        );
    }

    #[test]
    fn flags_outside_the_allowlist_are_never_smuggled_through() {
        for (line, says) in [
            ("pr view 3 --web", "--web"),
            ("pr checks --watch", "--watch"),
            ("issue list --hostname=evil", "--hostname"),
            ("pr view -- --web", "--web"),
            ("pr create --reviewer someone", "--reviewer"),
            ("pr edit --add-label bug", "--add-label"),
            ("pr view 3 -q $ENV", "environment"),
            ("api repos/acme/app/issues -f title=x", "-f"),
            ("api repos/acme/app/issues --input body.json", "--input"),
            ("api repos/acme/app -H X-HTTP-Method-Override:POST", "-H"),
            ("api repos/acme/app/pulls -XPOST", "only GET"),
        ] {
            assert!(refusal(line).contains(says), "{line}: {}", refusal(line));
        }
    }

    #[test]
    fn another_repo_is_refused_however_it_is_named() {
        for line in [
            "issue view 1 --repo=other/app",
            "pr view https://github.com/other/app/pull/1",
            "api repos/other/app",
            "api repos/acme/app/../../other/app",
            "api https://api.github.com/repos/acme/app",
            "api graphql",
        ] {
            assert!(
                refusal(line).contains("acme/app"),
                "{line}: {}",
                refusal(line)
            );
        }
    }

    #[test]
    fn api_reads_under_this_repo_pass_as_get() {
        assert_eq!(
            passed("api /repos/{owner}/{repo}/pulls?state=open --paginate"),
            "api repos/acme/app/pulls?state=open --method GET --paginate"
        );
        assert_eq!(
            passed("api repos/acme/app -X get"),
            "api repos/acme/app --method GET"
        );
    }

    #[test]
    fn pr_changes_target_this_sessions_branch_only() {
        let created = call("pr create --fill --head orch/work --base main").unwrap();
        assert_eq!(
            created.args.join(" "),
            "pr create --head orch/work --base main --fill --repo acme/app"
        );
        assert_eq!(created.change, Some(PrChange::Create));
        let edited = call("pr edit https://github.com/acme/app/pull/42 -t New").unwrap();
        assert_eq!(
            edited.args.join(" "),
            "pr edit orch/work --title New --repo acme/app"
        );
        assert_eq!(edited.change, Some(PrChange::Edit { pr: Some(42) }));
        assert!(refusal("pr create --head other").contains("Branch orch/work"));
        assert!(refusal("pr ready other-branch").contains("Session's PR"));
    }

    #[test]
    fn writes_other_than_pr_create_and_edit_are_refused() {
        for line in [
            "pr merge 42",
            "pr close 42",
            "pr comment 42",
            "issue comment 1",
            "issue edit 1",
            "label create bug",
            "repo delete",
            "auth token",
        ] {
            assert!(refusal(line).contains("doesn't run"), "{line}");
        }
    }
}
