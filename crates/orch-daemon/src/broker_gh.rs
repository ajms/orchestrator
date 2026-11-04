use orch_config::{ReadableOwners, is_github_owner};

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
        Self::from_parts(named.owner, named.name)
    }

    fn from_argument(text: &str) -> Option<Self> {
        let named = Named::parse(text)?;
        if !named.on_github() || named.host.is_none() && !named.rest.is_empty() {
            return None;
        }
        Self::from_parts(named.owner, named.name)
    }

    fn from_parts(owner: &str, name: &str) -> Option<Self> {
        let valid = |part: &str| {
            !matches!(part, "" | "." | "..")
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        (is_github_owner(owner) && valid(name)).then(|| GitHubRepo {
            owner: owner.into(),
            name: name.into(),
        })
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub(crate) fn same(&self, other: &GitHubRepo) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }

    fn owns_url(&self, url: &str) -> bool {
        Named::parse(url).is_some_and(|named| named.host.is_some() && named.names(self))
    }
}

pub(crate) struct Limits<'a> {
    pub(crate) repo: &'a GitHubRepo,
    pub(crate) branch: &'a str,
    pub(crate) base: &'a str,
    pub(crate) readable_owners: &'a ReadableOwners,
}

impl Limits<'_> {
    fn target(&self, named: &[String], writes: bool) -> Result<GitHubRepo, String> {
        let mut target: Option<GitHubRepo> = None;
        for text in named {
            let repo = match GitHubRepo::from_argument(text) {
                Some(repo) if repo.same(self.repo) => self.repo.clone(),
                Some(repo) if !writes && self.readable_owners.reads(&repo.owner) => repo,
                _ => return Err(another_repo(text, self, writes)),
            };
            match &target {
                Some(target) if !target.same(&repo) => {
                    return Err(format!(
                        "the call names both {} and {}",
                        target.slug(),
                        repo.slug()
                    ));
                }
                _ => target = Some(repo),
            }
        }
        Ok(target.unwrap_or_else(|| self.repo.clone()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    PrCreate,
    PrEdit { pr: Option<u64> },
    Issue,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GhCall {
    pub(crate) args: Vec<String>,
    pub(crate) said: String,
    pub(crate) change: Option<Change>,
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

const READ: bool = false;
const WRITE: bool = true;

const REPO: Flag = value("repo", Some('R'));
const OUTPUT: &[Flag] = &[
    REPO,
    value("json", None),
    value("jq", Some('q')),
    value("template", Some('t')),
];
const REPO_VIEW: &[Flag] = &[
    value("json", None),
    value("jq", Some('q')),
    value("template", Some('t')),
    value("branch", Some('b')),
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
const ISSUE_CREATE: &[Flag] = &[
    REPO,
    value("title", Some('t')),
    value("body", Some('b')),
    value("label", Some('l')),
    value("assignee", Some('a')),
];
const ISSUE_COMMENT: &[Flag] = &[REPO, value("body", Some('b'))];
const ISSUE_EDIT: &[Flag] = &[
    REPO,
    value("title", Some('t')),
    value("body", Some('b')),
    value("add-label", None),
    value("remove-label", None),
    value("add-assignee", None),
    value("remove-assignee", None),
];
const ISSUE_CLOSE: &[Flag] = &[
    REPO,
    value("comment", Some('c')),
    value("reason", Some('r')),
];
const ISSUE_REOPEN: &[Flag] = &[REPO, value("comment", Some('c'))];
const FIELDS: &[Flag] = &[value("field", Some('F')), value("raw-field", Some('f'))];

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
        return api(&args[1..], limits);
    }
    if (command, sub) == ("repo", "clone") {
        return Err(
            "the Broker doesn't run `gh repo clone`; use `git clone https://github.com/<owner>/<repo>`, which the Broker reads for owners in [broker] readable_owners"
                .into(),
        );
    }
    let (tables, at_most, writes): (&[&[Flag]], usize, bool) = match (command, sub) {
        ("issue", "view") => (&[OUTPUT, ISSUE_VIEW], 1, READ),
        ("issue", "list") => (&[OUTPUT, ISSUE_LIST], 0, READ),
        ("pr", "view") => (&[OUTPUT, ISSUE_VIEW], 1, READ),
        ("pr", "list") => (&[OUTPUT, PR_LIST], 0, READ),
        ("pr", "diff") => (&[PR_DIFF], 1, READ),
        ("pr", "checks") => (&[OUTPUT, PR_CHECKS], 1, READ),
        ("run", "view") => (&[OUTPUT, RUN_VIEW], 1, READ),
        ("run", "list") => (&[OUTPUT, RUN_LIST], 0, READ),
        ("repo", "view") => (&[REPO_VIEW], 1, READ),
        ("pr", "create") => (&[PR_CREATE], 0, WRITE),
        ("pr", "edit") => (&[PR_EDIT], 1, WRITE),
        ("pr", "ready") => (&[PR_READY], 1, WRITE),
        ("issue", "create") => (&[ISSUE_CREATE], 0, WRITE),
        ("issue", "comment") => (&[ISSUE_COMMENT], 1, WRITE),
        ("issue", "edit") => (&[ISSUE_EDIT], 1, WRITE),
        ("issue", "close") => (&[ISSUE_CLOSE], 1, WRITE),
        ("issue", "reopen") => (&[ISSUE_REOPEN], 1, WRITE),
        _ => return Err(refused(&what)),
    };
    let mut parsed = parse(&args[2..], tables, &what)?;
    if parsed.positionals.len() > at_most {
        return Err(format!("too many arguments for {what}"));
    }
    let said = match parsed.positionals.first() {
        Some(target) => format!("{what} {target}"),
        None => what.clone(),
    };
    let mut named = parsed.take("repo");
    if command == "repo" {
        named.extend(parsed.positionals.pop());
    }
    named.extend(
        parsed
            .positionals
            .iter()
            .filter(|arg| arg.contains("://") || arg.starts_with("github.com/"))
            .cloned(),
    );
    let target = limits.target(&named, writes)?;
    no_env_in_jq(&parsed)?;
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
            (strings(&fixed), Some(Change::PrCreate))
        }
        "edit" | "ready" if command == "pr" => {
            let pr = own_pr(parsed.positionals.pop(), limits)?;
            if parsed.take("base").iter().any(|base| base != limits.base) {
                return Err(fixed_base(limits.base));
            }
            (
                strings(&["pr", sub, limits.branch]),
                Some(Change::PrEdit { pr }),
            )
        }
        "create" | "comment" | "edit" | "close" | "reopen" if command == "issue" => {
            only_the_user_assigned(&parsed)?;
            (strings(&[command, sub]), Some(Change::Issue))
        }
        "view" if command == "repo" => (strings(&["repo", "view", &target.slug()]), None),
        _ => (strings(&[command, sub]), None),
    };
    args.extend(parsed.args());
    if command != "repo" {
        args.extend(strings(&["--repo", &target.slug()]));
    }
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

fn only_the_user_assigned(parsed: &Parsed) -> Result<(), String> {
    let assigns = ["assignee", "add-assignee", "remove-assignee"];
    match parsed
        .flags
        .iter()
        .filter(|(name, _)| assigns.contains(name))
        .all(|(_, value)| value.as_deref() == Some("@me"))
    {
        true => Ok(()),
        false => Err("the Broker assigns issues only to the user, as @me".into()),
    }
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

fn api(args: &[String], limits: &Limits) -> Result<GhCall, String> {
    let repo = limits.repo;
    let mut parsed = parse(args, &[API, FIELDS], "gh api")?;
    let [endpoint] = parsed.positionals.as_slice() else {
        return Err("gh api takes one endpoint".into());
    };
    let endpoint = endpoint
        .trim_start_matches('/')
        .replace("{owner}", &repo.owner)
        .replace("{repo}", &repo.name);
    let path = endpoint.split('?').next().unwrap_or_default();
    let posts = parsed
        .take("method")
        .iter()
        .any(|method| !method.eq_ignore_ascii_case("GET"));
    let named = path.strip_prefix("repos/").and_then(|rest| {
        let mut parts = rest.splitn(3, '/');
        let (owner, name) = (parts.next()?, parts.next()?);
        let prefix = "repos/".len() + owner.len() + 1 + name.len();
        Some((GitHubRepo::from_parts(owner, name)?, prefix))
    });
    let safe = !(path.contains("..") || path.contains('%') || path.contains('\\'));
    let Some((_, prefix)) = named.filter(|(named, _)| {
        safe && (named.same(repo) || !posts && limits.readable_owners.reads(&named.owner))
    }) else {
        return Err(format!(
            "the Broker's gh api reaches only repos/{} on GitHub, and GET under repos of owners in [broker] readable_owners",
            repo.slug()
        ));
    };
    let change = match posts {
        true => Some(issue_link(&parsed, &endpoint[prefix..])?),
        false if parsed.flags.iter().any(|(name, _)| field(name)) => {
            return Err(
                "the Broker passes -f/-F fields to gh api only to link sub-issues and blocking issues"
                    .into(),
            );
        }
        false => None,
    };
    no_env_in_jq(&parsed)?;
    let said = format!("gh api {endpoint}");
    parsed.positionals.clear();
    let method = match change {
        Some(_) => "POST",
        None => "GET",
    };
    let mut args = strings(&["api", &endpoint, "--method", method]);
    args.extend(parsed.args());
    Ok(GhCall { args, said, change })
}

fn field(name: &str) -> bool {
    matches!(name, "field" | "raw-field")
}

fn issue_link(parsed: &Parsed, path: &str) -> Result<Change, String> {
    let refused = || {
        "the Broker passes only GET requests to gh api, and POSTs that link sub-issues or blocking issues"
            .to_string()
    };
    let link = path.strip_prefix("/issues/").and_then(|rest| {
        let (number, link) = rest.split_once('/')?;
        number.bytes().all(|b| b.is_ascii_digit()).then_some(link)
    });
    let id_field = match link {
        Some("sub_issues") => "sub_issue_id",
        Some("dependencies/blocked_by") => "issue_id",
        _ => return Err(refused()),
    };
    let fields: Vec<_> = parsed
        .flags
        .iter()
        .filter(|(name, _)| field(name))
        .collect();
    let valid = |value: &str| {
        value.split_once('=').is_some_and(|(name, id)| {
            name == id_field && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
        })
    };
    match fields.as_slice() {
        [(_, Some(value))] if valid(value) => Ok(Change::Issue),
        _ => Err(format!(
            "linking issues takes exactly one numeric {id_field} field"
        )),
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

fn refused(what: &str) -> String {
    format!(
        "the Broker doesn't run `{what}`: it passes reads of this Repo, creating or editing this Session's PR, and filing or updating issues"
    )
}

fn another_repo(text: &str, limits: &Limits, writes: bool) -> String {
    let slug = limits.repo.slug();
    match writes {
        true => format!("the Broker changes only this Repo, {slug}, not {text}"),
        false => format!(
            "the Broker reads only this Repo, {slug}, and repos of owners in [broker] readable_owners, not {text}"
        ),
    }
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
        let readable = ReadableOwners::new(vec!["libs".into()]).unwrap();
        let limits = Limits {
            repo: &repo,
            branch: "orch/work",
            base: "main",
            readable_owners: &readable,
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
    fn repo_view_pins_this_repo_as_its_argument() {
        assert_eq!(passed("repo view"), "repo view acme/app");
        assert_eq!(
            passed("repo view https://github.com/ACME/app -q.name --json=name"),
            "repo view acme/app --jq .name --json name"
        );
        assert_eq!(
            passed("repo view acme/app -b dev"),
            "repo view acme/app --branch dev"
        );
    }

    #[test]
    fn flags_outside_the_allowlist_are_never_smuggled_through() {
        for (line, says) in [
            ("pr view 3 --web", "--web"),
            ("repo view --web", "--web"),
            ("repo view -R acme/app", "-R"),
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
            "repo view other/app",
            "repo view https://github.com/other/app",
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
    fn reads_of_a_readable_owners_repo_pass_with_that_repo_pinned() {
        for (line, args) in [
            (
                "issue view 1 --repo=libs/core",
                "issue view 1 --repo libs/core",
            ),
            ("pr diff 3 -R LIBS/core", "pr diff 3 --repo LIBS/core"),
            (
                "pr view https://github.com/libs/core/pull/1 --json title",
                "pr view https://github.com/libs/core/pull/1 --json title --repo libs/core",
            ),
            (
                "issue list -R libs/core -L 3",
                "issue list --limit 3 --repo libs/core",
            ),
            (
                "repo view libs/core --json name",
                "repo view libs/core --json name",
            ),
            (
                "repo view https://github.com/libs/core",
                "repo view libs/core",
            ),
            (
                "api /repos/libs/core/contents/README.md",
                "api repos/libs/core/contents/README.md --method GET",
            ),
        ] {
            let call = call(line).unwrap();
            assert_eq!(call.args.join(" "), args, "{line}");
            assert_eq!(call.change, None, "{line}");
        }
    }

    #[test]
    fn a_readable_owners_repo_is_never_changed() {
        for line in [
            "issue comment 1 -b x -R libs/core",
            "issue create -t x --repo libs/core",
            "issue close https://github.com/libs/core/issues/1",
            "pr create --fill -R libs/core",
            "pr edit https://github.com/libs/core/pull/1 -t x",
            "api -X POST repos/libs/core/issues/3/sub_issues -F sub_issue_id=1",
        ] {
            assert!(
                refusal(line).contains("acme/app"),
                "{line}: {}",
                refusal(line)
            );
        }
    }

    #[test]
    fn a_read_naming_two_repos_or_no_repo_name_is_refused() {
        for line in [
            "issue view https://github.com/libs/core/issues/1 --repo acme/app",
            "pr view https://github.com/acme/app/pull/1 -R libs/core",
            "api repos/libs",
            "api repos/libs/core/../../other/x",
            "repo view libs",
        ] {
            assert!(call(line).is_err(), "{line} went through");
        }
    }

    #[test]
    fn repo_clone_points_at_git_clone() {
        assert!(refusal("repo clone libs/core").contains("git clone"));
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
        assert_eq!(created.change, Some(Change::PrCreate));
        let edited = call("pr edit https://github.com/acme/app/pull/42 -t New").unwrap();
        assert_eq!(
            edited.args.join(" "),
            "pr edit orch/work --title New --repo acme/app"
        );
        assert_eq!(edited.change, Some(Change::PrEdit { pr: Some(42) }));
        assert!(refusal("pr create --head other").contains("Branch orch/work"));
        assert!(refusal("pr ready other-branch").contains("Session's PR"));
    }

    #[test]
    fn writes_other_than_pr_and_issue_changes_are_refused() {
        for line in [
            "pr merge 42",
            "pr close 42",
            "pr comment 42",
            "issue delete 1",
            "issue transfer 1 other/app",
            "issue lock 1",
            "issue pin 1",
            "issue develop 1",
            "label create bug",
            "repo delete",
            "auth token",
        ] {
            assert!(refusal(line).contains("doesn't run"), "{line}");
        }
    }

    #[test]
    fn issue_writes_pass_with_normalised_flags_and_this_repo_pinned() {
        for (line, args) in [
            (
                "issue create -t Bug -b Why -l bug -a @me",
                "issue create --title Bug --body Why --label bug --assignee @me --repo acme/app",
            ),
            (
                "issue comment 7 -b hi",
                "issue comment 7 --body hi --repo acme/app",
            ),
            (
                "issue edit 7 -t New --add-label x --remove-label y --add-assignee @me --remove-assignee @me",
                "issue edit 7 --title New --add-label x --remove-label y --add-assignee @me --remove-assignee @me --repo acme/app",
            ),
            (
                "issue close https://github.com/acme/app/issues/7 -c done -r completed",
                "issue close https://github.com/acme/app/issues/7 --comment done --reason completed --repo acme/app",
            ),
            (
                "issue reopen 7 -c back",
                "issue reopen 7 --comment back --repo acme/app",
            ),
        ] {
            let call = call(line).unwrap();
            assert_eq!(call.args.join(" "), args);
            assert_eq!(call.change, Some(Change::Issue), "{line}");
        }
    }

    #[test]
    fn issue_writes_assign_only_the_user_and_stay_in_this_repo() {
        for (line, says) in [
            ("issue create -t x -a someone", "@me"),
            ("issue edit 7 --add-assignee bob", "@me"),
            ("issue edit 7 --remove-assignee @me,bob", "@me"),
            ("issue comment 7 --edit-last", "--edit-last"),
            ("issue create --web", "--web"),
            ("issue edit 7 --milestone v1", "--milestone"),
            ("issue create -t x --project Roadmap", "--project"),
            ("issue comment 7 8 -b x", "too many"),
            (
                "issue comment https://github.com/other/app/issues/1 -b x",
                "acme/app",
            ),
            ("issue close 1 -R other/app", "acme/app"),
        ] {
            assert!(refusal(line).contains(says), "{line}: {}", refusal(line));
        }
    }

    #[test]
    fn api_writes_only_link_sub_issues_and_blocking_issues() {
        for (line, args) in [
            (
                "api --method POST repos/acme/app/issues/3/sub_issues -F sub_issue_id=123",
                "api repos/acme/app/issues/3/sub_issues --method POST --field sub_issue_id=123",
            ),
            (
                "api -X post /repos/{owner}/{repo}/issues/3/dependencies/blocked_by -F issue_id=9",
                "api repos/acme/app/issues/3/dependencies/blocked_by --method POST --field issue_id=9",
            ),
        ] {
            let call = call(line).unwrap();
            assert_eq!(call.args.join(" "), args);
            assert_eq!(call.change, Some(Change::Issue), "{line}");
        }
        assert_eq!(
            call("api repos/acme/app/issues/3/sub_issues")
                .unwrap()
                .change,
            None
        );
        for line in [
            "api -X POST repos/acme/app/issues -F title=x",
            "api -X POST repos/acme/app/issues/3/sub_issues/priority -F sub_issue_id=1",
            "api -X DELETE repos/acme/app/issues/3/sub_issue -F sub_issue_id=1",
            "api -X POST repos/acme/app/issues/3/sub_issues -F sub_issue_id=@id.txt",
            "api -X POST repos/acme/app/issues/3/sub_issues -F title=x",
            "api -X POST repos/acme/app/issues/3/dependencies/blocked_by -F sub_issue_id=1",
            "api -X POST repos/acme/app/issues/x/sub_issues -F sub_issue_id=1",
            "api repos/acme/app/issues/3/sub_issues -F sub_issue_id=1",
            "api -X POST repos/other/app/issues/3/sub_issues -F sub_issue_id=1",
        ] {
            assert!(call(line).is_err(), "{line}");
        }
    }
}
