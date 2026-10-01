# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues in `ajms/orchestrator`. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

`gh` infers the repo when run inside a clone. The `gh api` calls below name it explicitly.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments` then keep only `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE` (drop `OWNER`/`MEMBER`/`COLLABORATOR`).
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue; its tickets are native GitHub **sub-issues** of it. The labels `wayfinder:map`, `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling` and `wayfinder:task` already exist.

Sub-issue and dependency endpoints take an issue's **database id**, not its `#number` or `node_id`:

```sh
gh api repos/ajms/orchestrator/issues/<n> --jq .id
```

### Map

```sh
gh issue create --label wayfinder:map --title "<effort name>" --body-file map.md
```

### Child ticket

Create the issue with `Part of #<map>` as the first line of the body, then link it as a sub-issue:

```sh
gh issue create --label wayfinder:<type> --title "<question as a name>" --body-file ticket.md
gh api --method POST repos/ajms/orchestrator/issues/<map>/sub_issues \
  -F sub_issue_id="$(gh api repos/ajms/orchestrator/issues/<child> --jq .id)"
```

Sub-issue order is map order.

### Blocking

Native issue dependencies, wired in a second pass once every ticket has a number:

```sh
gh api --method POST repos/ajms/orchestrator/issues/<blocked>/dependencies/blocked_by \
  -F issue_id="$(gh api repos/ajms/orchestrator/issues/<blocker> --jq .id)"
```

A ticket is unblocked when every blocker is closed.

### Frontier query

Open, unassigned sub-issues of the map with no open blocker, in map order; the first one wins:

```sh
gh api graphql -F map=<map> -f query='
query($map: Int!) {
  repository(owner: "ajms", name: "orchestrator") {
    issue(number: $map) {
      subIssues(first: 100) {
        nodes {
          number
          title
          state
          url
          labels(first: 10) { nodes { name } }
          assignees(first: 1) { totalCount }
          blockedBy(first: 50) { nodes { state } }
        }
      }
    }
  }
}' --jq '.data.repository.issue.subIssues.nodes[]
  | select(.state == "OPEN" and .assignees.totalCount == 0)
  | select([.blockedBy.nodes[] | select(.state == "OPEN")] | length == 0)
  | {number, title, url, type: ([.labels.nodes[].name | select(startswith("wayfinder:"))][0])}'
```

### Claim

The session's first write, before any other work:

```sh
gh issue edit <n> --add-assignee @me
```

### Resolve

```sh
gh issue comment <n> --body-file answer.md
gh issue close <n>
gh issue view <map> --json body --jq .body > map.md
# append "- [<ticket title>](<ticket url>): <one-line gist>" under "## Decisions so far"
gh issue edit <map> --body-file map.md
```

Other sessions may edit the map concurrently, so re-read its body right before writing it back.
