# Pushing and gh from inside the Sandbox

Research for [#132](https://github.com/ajms/orchestrator/issues/132), part of map [#126](https://github.com/ajms/orchestrator/issues/126). Input: [#129](https://github.com/ajms/orchestrator/issues/129) (the `$HOME` allowlist hides `~/.ssh`, `~/.config/gh`, the keyring and D-Bus, and drops `SSH_AUTH_SOCK`).

The question: what are the facts for giving a sandboxed Claude **one** way to push and to use `gh`, and nothing else? Facts only. The decision belongs to [#133](https://github.com/ajms/orchestrator/issues/133).

## Sources

- **Host:** Arch Linux, kernel 7.2.9, `openssh 10.6p1-1`, `bubblewrap 0.13.0-1`, `github-cli 2.102.0-1`, `git 2.56.0-1` (`pacman -Q`).
- **OpenSSH:** `ssh-add(1)` of 10.6p1 (cited as *ssh-add(1)*). Release notes 8.9 (https://www.openssh.org/txt/release-8.9). The design note https://www.openssh.org/agent-restrict.html (cited as *agent-restrict*). `ssh-agent.c` at tags `V_8_9_P1` and `V_10_6_P1` of openssh/openssh-portable (cited as *ssh-agent.c*).
- **GitHub docs**, fetched 2026-10-13 through `docs.github.com/api/article/body`:
  - *Managing your personal access tokens* (cited as *PAT docs*)
  - *Permissions required for fine-grained personal access tokens* (cited as *FG permissions*)
  - *Managing deploy keys* (cited as *deploy keys*)
- **GitHub API:** `api.github.com/meta` (GitHub's SSH host keys), the repo's rulesets (`gh api repos/ajms/orchestrator/rulesets/<id>`, read only).
- **gh:** `gh help environment` of 2.102.0. Source at tag `v2.102.0`: `internal/config/config.go` (`ActiveToken`) and `internal/authflow/flow.go` (login scopes).
- **Claude Code docs:** https://code.claude.com/docs/en/sandboxing (cited as *CC sandboxing*).
- **Distro packages:** sources.debian.org API, the Launchpad API, mdapi.fedoraproject.org, and the CentOS Stream `openssh.spec` on gitlab.com.
- **orch on `main` 408402e:** `crates/orch-holder/src/{protocol,holder,paths}.rs`, `crates/orch-git/src/landing.rs`, `crates/orch-daemon/src/pr.rs`, `crates/orch-agent/src/guard/{git,gh}.rs`.
- **Experiments in `$TMPDIR`** (section 5): a throwaway `ssh-agent` with two freshly generated throwaway ed25519 keys, its socket bound into `bwrap`, and `gh` with a fake `GH_TOKEN`. No real key, agent or token was used, and no key or token was created on GitHub. The user's tool guard blocks `ssh-add -l`/`-L` and `gh auth …` outright, so listing was observed through `ssh -v` instead.

## Short answer

1. **ssh-agent works from inside bwrap.** orch can start a private `ssh-agent -a <path>` holding one key added with `ssh-add -h github.com`, and bind its socket (or better, its directory) into the Sandbox. From inside, `ssh` sees the key for github.com and **not** for any other host. The agent **refuses** to use the constrained key for anything but an SSH user-auth signature on a connection bound to a github.com host key. Arbitrary signing (`ssh-keygen -Y sign`) is refused.
2. **The destination constraint limits the host, not the repo.** On github.com the key carries whatever the key is attached to. If it is the user's account key, the Agent can push to **every repo the account can write**, and `ssh -T` works. Only a **deploy key** limits SSH to one repo, and a deploy key can't be reused across repos.
3. **The Agent can disrupt the agent.** From inside it can `ssh-add -D` (remove all keys) or lock the agent. That's denial of service, not theft: the private key never leaves the agent.
4. **Two practical blockers for SSH inside the Sandbox:**
   - The user's remote uses `~/.ssh/config` aliases (`git@github.com-read:`, `git@github.com-write:`). `~/.ssh` is hidden, so those names don't resolve. `.git/config` is read-only there, so the Agent can't rewrite the remote.
   - `ssh` reads `known_hosts` from the passwd home (the tmpfs), so there is none. Without a host key the push fails.

   orch would have to supply both: a host alias mapping or a URL rewrite, and a `known_hosts` with GitHub's keys.
5. **Claude Code's own Bash sandbox blocks plain ssh.** Nested inside orch's Sandbox, a sandboxed Bash command has no route out except Claude Code's HTTP/SOCKS proxy, and `ssh` ignores the proxy. `git push` over ssh needs Claude Code's `excludedCommands` or an unsandboxed retry. `git` over HTTPS and `gh` go through the proxy.
6. **`gh` honours `GH_TOKEN` with no `~/.config/gh`.** It needs no keyring or D-Bus and writes nothing under `$HOME` (tested).
7. **A fine-grained PAT can be limited to one repo** (one resource owner, "Only select repositories"). The permissions are:
   - `Metadata: read` (always), `Pull requests: write` (create and edit PRs), `Issues: write` (issues, sub-issues, dependencies). `Contents: read` is enough unless the token also pushes.
   - Merging a PR needs `Contents: write`. Without it the token can't merge or push.
   - The **Checks API is not available** to fine-grained PATs (a documented gap).
8. **A fine-grained PAT is minted by hand in the web UI.** The docs describe no API for creating one. A pre-filled URL can set name, expiry (1–366 days or none) and permissions, but **not the repository selection**. `gh auth login`'s device flow yields an OAuth token with `repo`, `read:org` and `gist` scopes, which is account-wide.
9. **The weak point is `GH_TOKEN` itself.** It sits in the Agent's environment in clear. The network is open, so the Agent can copy it out and use it until it expires or is revoked. The SSH key, by contrast, is usable only while the socket is reachable.
10. **Broker alternative:** orch already pushes (`orch-git` `push`) and runs `gh pr …` from the Daemon with the user's own credentials. The smallest broker is one new Holder frame plus an `orch` subcommand that asks the Daemon to push **this Session's branch** (or open or update its PR). The Daemon derives the refspec and remote, not the Agent. Constraint: the Daemon runs unsandboxed git on a repo the Agent can write, so it must pin `core.hooksPath`, `core.sshCommand` and friends on the command line.

## 1. A private ssh-agent with a destination-constrained key

### What `-h` does (OpenSSH ≥ 8.9)

- *ssh-add(1)*, `-h destination_constraint`: "constrain them to be usable only through specific hosts or to specific destinations. … ‘[user@]dest-hostname’ permit use of the key only from the origin host (the one running ssh-agent(1)) to the listed destination host".
- "Hosts are identified by their host keys, and are looked up from known hosts files by ssh-add." `-H hostkey_file` names those files; the default is `~/.ssh/known_hosts` and friends. "Destination constraints were added in OpenSSH release 8.9."
- *ssh-add(1)* caveat: constraints "can only be enforced by ssh-agent(1) when a key is used, or when it is forwarded by a cooperating ssh(1)".
- *agent-restrict*: the binding is "between the signature and the server hostkey", so "the final destination cannot be forged".
- Release 8.9 ("ssh(1), sshd(8), ssh-add(1), ssh-agent(1): add a system for restricting forwarding and use of keys added to ssh-agent(1)") points to *agent-restrict* and the PROTOCOL files.

### How the agent enforces it (*ssh-agent.c*, identical at `V_8_9_P1` and `V_10_6_P1`)

- **Listing.** `process_request_identities` skips any identity for which `identity_permitted(id, e, NULL, NULL, NULL)` fails. A client bound to gitlab.com's host key gets an empty list.
- **Signing.** `process_sign_request2`, for a key with `ndest_constraints != 0`:
  - It refuses on a connection with no session binding: "refusing use of destination-constrained key to sign on unbound connection".
  - It refuses data that isn't an SSH user-auth request: "… to sign an unidentified signature".
  - It then checks `identity_permitted`, that the session ID is the most recently bound one, and that the host key in the signature matches the bound one.
- So a local client that skips `session-bind@openssh.com` (a custom client, `ssh-keygen -Y sign`) **can't** use a constrained key. This is stronger than the man page's "when a key is used" wording suggests. (In `identity_permitted` itself there is a `return 0; /* local use */` branch for unbound sockets, but the sign path refuses before it matters.)

### What it does not limit

- **The repo.** GitHub authenticates the key, not the path. A key on the user's account authorises every repo the account can push to (*PAT docs* says the same for classic tokens: "all repositories within the organizations that you have access to, as well as all personal repositories").
- **A deploy key is per repo.** *deploy keys*:
  - "a deploy key, which is an SSH key that grants access to a single repository"
  - "You can't reuse a deploy key for multiple repositories"
  - "Deploy keys are read-only by default, but you can give them write access"
  - "Deploy keys with write access can perform the same actions as … a collaborator on a personal repository."
- **What on the repo.** Any ref, subject to rulesets. This repo has two rulesets on `refs/heads/main`:
  - `main-1`: `deletion` and `non_fast_forward`, no bypass.
  - `main-2`: `required_signatures` and `pull_request`, with `RepositoryRole 5` (admin) bypassing **always**.

  The owner's key (or token) is admin, so it bypasses `main-2`. An Agent holding it could fast-forward `main` directly, with no PR. Whether a write deploy key counts as that bypass role was not verified.
- **`ssh -T git@github.com`** works with any key GitHub accepts. It is harmless (no shell) but proves identity.
- **Agent management.** Anyone who can `connect()` to the socket can remove keys (`ssh-add -d/-D`), lock the agent (`ssh-add -x`) or add keys of their own. *agent-restrict* says "attempts to remove or adjust options on a restricted key will be refused anywhere other than on the origin machine". The Sandbox **is** the origin machine (unforwarded). Measured: `ssh-add -D` from inside bwrap through a read-only directory bind removed the key.

### Keys that need a touch or a confirmation

- The signing happens in the agent, outside the Sandbox. A FIDO (`-sk`) key still asks for a touch on each use. Per the auto-memory, the user's push key is such a key. The touch is then a per-push human confirmation the Agent can't skip, but it also means unattended pushes block.
- `ssh-add -c` (confirm each use) runs `ssh-askpass` from the agent's side, which needs a display there, not in the Sandbox.

### Plumbing an orch-owned agent

- `ssh-agent -a <path>` puts the socket at a chosen path. `ssh-agent -D` keeps it in the foreground, so a Holder could own it as a child. `ssh-add -H <file> -h github.com <key>` needs GitHub's host keys at add time. They are published at `api.github.com/meta` (`ssh_keys`, ed25519 `SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU`). GitHub rotated its RSA host key in 2023, so a pinned list can go stale.
- **Bind the directory, not the socket file.** A file bind pins the inode: if the agent restarts, the Sandbox keeps a dead socket. A read-only bind of the socket or its directory does not stop `connect()` (measured).
- Inside, set `SSH_AUTH_SOCK` to the bound path. #129 currently unsets it.
- **known_hosts.** `ssh` takes `~` from the passwd entry, not `$HOME`. Measured: with `HOME=/home` it still looked for `/home/albert/.ssh/id_*`. Under #129's tmpfs that is empty, so orch has to bind a `known_hosts` at `~/.ssh/known_hosts` (read-only) or set `core.sshCommand`/`GIT_SSH_COMMAND` with `-o UserKnownHostsFile=…`. Otherwise a non-interactive push fails host-key verification.
- **Remote aliases.** This repo's `origin` is `git@github.com-read:ajms/orchestrator.git` (fetch) and `git@github.com-write:ajms/orchestrator.git` (push). The aliases live in the hidden `~/.ssh/config`. Ways to make them resolve without exposing the user's ssh config:
  - a generated `~/.ssh/config` bound read-only (`Host github.com-write` → `HostName github.com`, `IdentityAgent`, `UserKnownHostsFile`);
  - or git's `url.<base>.pushInsteadOf` passed through `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n`.

  Fetch over `github.com-read` breaks the same way. For a public repo it could be rewritten to anonymous HTTPS.
- **Per Session or per Repo.** Both work mechanically: an agent is just a socket. Per Session lets orch remove the socket with the Session. The key and its passphrase or touch are the user's in either case.

### Distro OpenSSH versions (need ≥ 8.9 for `-h`)

| distro | openssh | `-h` |
|---|---|---|
| Arch (this host) | 10.6p1 | yes |
| Debian 11 bullseye | 8.4p1 | **no** |
| Debian 12 bookworm | 9.2p1 (10.0p1 in backports) | yes |
| Debian 13 trixie | 10.0p1 | yes |
| Ubuntu 22.04 jammy | 8.9p1 | yes (first version) |
| Ubuntu 24.04 noble | 9.6p1 | yes |
| Ubuntu 25.10 questing | 10.0p1 | yes |
| Fedora 43 / 44 | 10.0p1 / 10.2p1 | yes |
| CentOS Stream 9 / 10 (RHEL 9.x, 10) | 9.9p1 | yes |

The client inside the Sandbox is the host's `ssh` (read-only `/usr`), so the client and agent versions match.

## 2. gh with a fine-grained token

### gh and `GH_TOKEN`

- `gh help environment`: "`GH_TOKEN`, `GITHUB_TOKEN` (in order of precedence): an authentication token that will be used when a command targets either `github.com` or a subdomain of `ghe.com`. Setting this avoids being prompted to authenticate and takes precedence over previously stored credentials."
- `config.go` `ActiveToken`: env or config first (`ghauth.TokenFromEnvOrConfig`), the keyring only if that is empty. The keyring needs the D-Bus secret service, which #129 removes, so env is the only route inside.
- Measured in bwrap with an empty tmpfs `$HOME`, no `XDG_CONFIG_HOME`, and `GH_TOKEN=github_pat_FAKE…`:
  - `gh issue view 132 -R ajms/orchestrator` reached `api.github.com/graphql` with the token and got `HTTP 401: Bad credentials`.
  - Without a token it says "populate the GH_TOKEN environment variable".
  - Nothing was written under `$HOME`. `GH_NO_UPDATE_NOTIFIER=1` keeps the update check quiet.
- `git push` over HTTPS with the same token needs a credential helper. `gh auth setup-git` installs `gh auth git-credential`, but that writes `.gitconfig`, which is read-only in the Sandbox. The alternative is to pass `credential.helper` through `GIT_CONFIG_*` env vars. Not tested: the user's guard blocks `gh auth`.

### Limiting a fine-grained PAT

- *PAT docs*: "Each token is limited to access resources owned by a single user or organization", and "can be further limited to only access specific repositories".
- For an org owner that requires approval, the token is `pending` and "will only be able to read public resources until it is approved". `ajms/orchestrator` is user-owned, so no approval is needed.
- **Permissions** (*FG permissions*):
  - `POST /repos/{o}/{r}/pulls` needs **Pull requests: write**. `PATCH` of a pull request (`gh pr edit --base`, which orch uses) is in the same group.
  - `POST /repos/{o}/{r}/issues`, `…/sub_issues` and `…/dependencies/blocked_by` need **Issues: write**. Issue comments, labels and assignees are listed under both Issues and Pull requests.
  - `PUT /repos/{o}/{r}/pulls/{n}/merge` is under **Contents: write**. So does `POST/PATCH/DELETE …/git/refs`. A token without `Contents: write` can't merge or move branches.
  - **Metadata: read** is mandatory. **Workflows: write** is a separate permission for changing `.github/workflows`.
  - **Commit statuses** has read and write.
- **Gaps** (*PAT docs*): fine-grained PATs can't "call the Checks API". They also can't contribute to repos where the user is only a collaborator, or reach several orgs at once. `gh pr checks`/`gh pr view --json statusCheckRollup` therefore may not see check runs. That was not measured.
- **GraphQL:** gh uses GraphQL for `pr` and `issue`. The 401 above shows the request goes to `/graphql` with the token. Whether every GraphQL field gh asks for is allowed to a fine-grained PAT was not measured.
- **Expiry:** 1–366 days or none. An org policy may cap it. Tokens unused for a year are removed.

### Minting and storing

- **By hand.** Settings → Developer settings → Fine-grained tokens. *PAT docs* describe no REST endpoint for creating one.
- **Pre-filled URL** (*PAT docs*): `https://github.com/settings/personal-access-tokens/new?name=…&description=…&target_name=…&expires_in=…&<permission>=<level>`. There's **no parameter for the repository list**, so the user still picks "Only select repositories" and the repo by hand. orch could print such a URL (for example `pull_requests=write&issues=write&contents=read`).
- **Device flow via gh.** `gh auth login` runs the OAuth device flow of gh's own OAuth app with `minimumScopes := {"repo", "read:org", "gist"}` (`flow.go`). That is a classic-scope, account-wide token. It isn't repo-limited.
- **GitHub App.** A user-owned App installed on one repo can give installation tokens or user-to-server tokens limited to that repo. But it needs an App registration and a private key outside the Sandbox, and the tokens expire (8 h user tokens with a refresh token). It's heavier than a hand-made PAT, and nothing in this research measured it.
- **Storing.** It's orch's choice where the token lives outside the Sandbox. Inside, it becomes an env var in clear. Claude Code's own sandbox can **mask** an env var: sandboxed commands see a sentinel, and the proxy substitutes the real value for allowed hosts. But that needs `network.tlsTerminate` (experimental) and user-scope settings (*CC sandboxing*, "Mask credentials"). The Claude process itself still holds the real value.

## 3. Claude Code's own sandbox, nested

From *CC sandboxing*: "Tools that ignore the proxy variables: plain `ssh`, most database drivers … can't connect, even to an allowed host", and "`git` over HTTPS … connect once their host is allowed". The documented way out is `excludedCommands` (runs unsandboxed, through the permission flow).

Measured inside this session's Claude Code sandbox: `ssh git@github.com` → "Could not resolve hostname github.com: Temporary failure in name resolution", with `HTTPS_PROXY`/`ALL_PROXY` set.

So, under orch's Sandbox with Claude's Bash sandbox also on (which #127 keeps working):
- An SSH push leaves Claude's sandbox (`excludedCommands` or the unsandboxed retry). It is then bounded only by orch's Sandbox.
- `gh` and HTTPS pushes stay in Claude's sandbox. `api.github.com` and `github.com` must be allowed domains.

## 4. Broker alternative (git push and gh through orch)

Facts about today's code:

- **The Holder socket is already inside the Sandbox** (#129: a private `$XDG_RUNTIME_DIR` with only this Session's Holder socket). `ORCH_HOLDER_SOCKET` and `ORCH_SESSION` are in the Agent's env (`holder.rs` `spawn_agent`, `paths.rs`). The `orch hook`/`orch tap` commands already talk to it from inside.
- **The frame set** (`protocol.rs` `ToHolder`) includes `Hook`, `Guard` (request/answer round trip through the Daemon, `GuardAnswer`) and `Tap`. It also includes `Input`, `Kill` and `Shutdown`. Anything inside the Sandbox can already send `Kill`/`Shutdown` to its own Holder, so the socket is not a privilege boundary today.
- **The Daemon already pushes and runs gh with the user's credentials**, unsandboxed:
  - `orch-git` `Repo::push(branch)` runs `git push -q -u origin refs/heads/<b>:refs/heads/<b>` (`landing.rs`).
  - `orch-daemon` `pr.rs` runs `gh pr list --head`, `gh pr edit --base`, `gh pr create`/`view` (`push_for_pr`, `open_pr`, `retarget_pr`, `refresh_pr`).
- **The Guard already classifies** `git push` refspecs (other refs, `--all`, `--mirror`, `--tags`) and `gh pr merge`/mutating `gh api` (`guard/git.rs`, `guard/gh.rs`).

The smallest viable shape:

- **One new request kind** on the Holder socket, for example `ToHolder::Broker { op }`, answered like `Guard` (the Holder relays it as a `HolderEvent`, the Daemon acts and answers).
- **Ops:**
  - `push`: the Daemon pushes **the Session's own branch**. The Daemon, not the Agent, picks the refspec, the remote and the flags (no force).
  - `open_pr` / `refresh_pr`: reuse the existing Daemon paths.
- **An `orch push` / `orch pr` CLI** inside the Sandbox, like `orch hook`. Claude would be told to use it instead of `git push`/`gh pr create`.
- **No credentials enter the Sandbox at all.** The user's existing `~/.ssh/config` aliases, FIDO touch and `gh` keyring keep working, because the Daemon runs outside.

Constraints a broker must respect:

- **Unsandboxed git on an Agent-written repo.**
  - #129 makes `.git/hooks` and `.git/config` read-only, but the rest of the git common dir is writable. That includes `.git/worktrees/<name>/`, which holds a `config.worktree` that git reads only when `extensions.worktreeConfig` is set. It is not set here; that key lives in the read-only `.git/config`.
  - The worktree's own files (`.gitattributes`) are Agent-controlled, but filter drivers need config to run.
  - So the Daemon should push with explicit overrides (`-c core.hooksPath=/dev/null`, `--no-verify`, a fixed `core.sshCommand`, no `-c` from the Agent). Today's Land push faces the same question.
- **`gh issue`** (wayfinder's sub-issues, dependencies, comments, labels) has a wide surface. A broker would either allowlist a few verbs or pass through any `gh issue …` argv. The latter is close to handing over the token, with orch's account-wide `gh` scopes.
- **Bypassing it.** With no credentials inside, the Agent can't push around the broker. But the network is open, so anything the Agent finds (for example a token printed by a tool) still works.
- **Per-push confirmation** stays possible. The Daemon could route a broker `push` through the existing Guard prompt UI, but that changes Claude's behaviour (a rollback reason of #95).

## 5. Experiments

All runs were in `$TMPDIR`. bwrap policy:

```
bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /home --tmpfs /tmp \
      --bind <scratch>/box /tmp/box --ro-bind <agent socket or dir> /tmp/agent… \
      --die-with-parent --setenv SSH_AUTH_SOCK /tmp/agent… -- sh -c …
```

Two throwaway keys: `tk1`, added with `ssh-add -H gh_hosts -h github.com` (host keys from `api.github.com/meta`), and `tk2`, added unconstrained.

| test (from inside bwrap) | result |
|---|---|
| `ssh-keygen -U -Y sign -f tk1.pub` (constrained, no session-bind) | `Signing ./msg failed: agent refused operation` |
| same with `tk2.pub` (unconstrained) | signature written |
| `ssh -v -T git@github.com` | `bound agent to hostkey`, `agent returned 1 keys`, `Offering public key: throwaway … agent`, then `Permission denied (publickey)` (key not on GitHub, as expected) |
| `ssh -v -T git@gitlab.com` | `bound agent to hostkey`, `agent contains no identities`: the constrained key is hidden |
| `git ls-remote git@github.com:ajms/orchestrator.git` via the agent | key offered, GitHub refuses it (unregistered), "Please make sure you have the correct access rights" |
| `ssh-add -D` through a **read-only** directory bind | `All identities removed.`; the key is gone from the agent afterwards |
| `ssh git@github.com` inside Claude Code's own Bash sandbox | `Could not resolve hostname github.com` |
| `gh issue view 132 -R ajms/orchestrator` with fake `GH_TOKEN`, empty tmpfs `$HOME` | `HTTP 401: Bad credentials (https://api.github.com/graphql)`; no files written |
| same without `GH_TOKEN` | "populate the GH_TOKEN environment variable" |

Not done:
- A real `git push` through a constrained key. That would need a key registered on GitHub, which this research doesn't create.
- `gh auth git-credential`. The user's guard blocks `gh auth`.
- `ssh-add -l` inside bwrap. The guard blocks it too; `ssh -v` showed the same thing.

## Open questions for #133

- **Account key or deploy key.** Account key: one key for all repos, but the Agent can push anywhere the user can, and as admin it bypasses `main-2`. Deploy key: one key per repo, which the user creates and registers per repo.
- **Does the token push too?** With `Contents: write` the token can push over HTTPS and merge PRs, which makes the ssh-agent redundant and widens the token. With `Contents: read` the token can't push or merge, and the ssh-agent is the only push path.
- **How SSH gets out of Claude's own sandbox** (`excludedCommands` for `git push`?), or prefer HTTPS so it stays proxied.
- **Is token exfiltration acceptable** (short expiry), or is that the reason to broker?
