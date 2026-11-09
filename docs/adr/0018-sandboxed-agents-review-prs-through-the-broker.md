# Sandboxed Agents review PRs through the Broker

This amends 0010, which kept a sandboxed Agent's PR changes to the Session's Branch and its `api` POSTs to two issue-link endpoints. A sandboxed Agent can now take part in review on any PR in its Repo, not just its own, under the user's identity:

- **comments:** `pr comment`, and `gh api -X POST repos/<owner>/<repo>/pulls/<n>/comments` for inline review comments and thread replies;
- **reviews:** `pr review` with `--comment` or `--request-changes`, which follow the Preset, and `--approve`, which always asks;
- **whole reviews:** `gh api -X POST repos/<owner>/<repo>/pulls/<n>/reviews --input <file>`, a summary and inline comments in one call, under the same rules as `pr review`.

Merging or closing PRs, resolving threads, re-requesting reviewers, and other repos stay refused.

A comment is the Agent speaking. An approval is the user's identity satisfying a review requirement on someone else's work, and an Agent reading a hostile PR or issue could be steered into it. So `--approve` prompts under every Preset, and it needs a PR number or URL. Its Guard is `(pr_approval, <owner>/<repo>#<n>)`, so *allow for Session* covers that one PR. Other review calls share a Guard of their own, on the Repo, so consent to edit the Session's PR never carries over to reviewing others.

## Considered options

- **Follow the Preset for approvals too.** Consistent, but under Presets that don't ask, nobody sees an approval go out under their name.
- **Refuse `--approve`.** The strongest guarantee, but the Agent can't finish a review the user asked for, and the user has to relay it by hand.
- **Allow the `pulls/<n>/reviews` endpoint.** It takes whole reviews with inline comments in one call, but its nested JSON needs `--input`, which the Broker refuses because it reads a host file. (Amended: now allowed. The shim reads the file inside the Sandbox and sends its text in the request, so `gh` only ever gets `--input -` and no host file is read.)
- **Only `pulls/<n>/comments`, one call per comment.** It needs no JSON, but a review with many findings lands as that many separate comments, with no summary.

## Consequences

- The `pulls/<n>/comments` fields are free text, so unlike the issue-link fields they can't be checked against a fixed shape. `gh api -F name=@path` reads a host file, so the Broker refuses a typed field whose value starts with `@`, and the Agent gives the text with `-f`. Without that check, a comment could carry any file the user can read out of the Sandbox.
- `gh` follows a PR or issue URL over `--repo`, and it accepts more spellings than the usual `https://`, such as `https:/owner/repo/pull/1`. Every positional that could be a URL, meaning any with a `:`, must name a repo the Broker allows. Git refs can't contain `:`, so branch names still pass.
- `gh api` writes are POST only.
- The Daemon parses a whole review and gives `gh` its own serialisation, so a repeated `event` key can't read as one verdict to the Broker and another to GitHub. Only `commit_id`, `body`, `event` and `comments` pass. `event` must be `COMMENT`, `REQUEST_CHANGES` or `APPROVE`; an `APPROVE` takes the approval Guard. A review with no `event` is refused, since it would stay pending and the Broker can't submit it.
