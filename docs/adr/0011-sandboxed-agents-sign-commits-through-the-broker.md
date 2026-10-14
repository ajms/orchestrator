# Sandboxed Agents sign commits through the Broker

A sandboxed Agent still runs `git commit` itself, but it never holds a signing key. Inside the Sandbox, orch sets `gpg.format=ssh`, `commit.gpgsign=true`, `user.signingkey` to the public half of a dedicated Agent key, and `gpg.ssh.program` to a shim. The shim forwards the payload over the Session's Holder socket. The Daemon signs it with `ssh-keygen -Y sign` using the private key, outside the Sandbox.

The Broker signs only a payload that parses as a commit object whose committer is the Agent identity, and refuses anything else. Signing never asks, whatever the Preset: a signature only matters once it is pushed, and the push still follows the Preset.

The key is a dedicated ed25519 SSH key with no passphrase. The user generates it by hand, names it in the global `[signing] key` config, and adds it to GitHub as a signing key, so Agent commits show as Verified. Every sandboxed commit uses the Agent identity: the user's name followed by ` (orch)`, with the user's email. Without a configured key, orch sets `commit.gpgsign=false` in the Sandbox, and Agent commits go through unsigned.

Unsandboxed Sessions and the Daemon's own commits (Landing, snapshots) keep the user's config and key.

## Considered options

- **A signing key inside the Sandbox.** This needs only config, but it breaks "credentials never enter the Sandbox" (0009, 0010): anything the Agent runs could copy the key out.
- **The Broker makes the commit.** The key stays outside just as well, but `git commit`, `--amend` and rebase stop working normally inside the Sandbox.
- **Signing with the user's own GPG key.** Agent commits would look exactly like the user's, and the key may need a passphrase or a touch for every commit.

## Consequences

- The Agent identity is fixed when the Agent starts and recorded in the Session's directory. The Broker checks commits against that record, not the user's current git config.
- The key file is masked with `/dev/null` inside the Sandbox, wherever it lives, so the Agent key can't leak from a Sandbox. A compromised Agent can only get its own commits signed, and only while its Session runs.
- The key name and the identity tell apart Agent commits and the user's.
- The user's host `commit.gpgsign` no longer breaks commits inside the Sandbox.
