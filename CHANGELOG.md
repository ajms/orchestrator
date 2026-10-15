# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1](https://github.com/ajms/orchestrator/compare/v0.5.0...v0.5.1) - 2026-10-15

### Fixed

- *(daemon)* a restarted Daemon no longer replays old attention

### Other

- keep injected git config out of the tui and git review fixtures

## [0.5.0](https://github.com/ajms/orchestrator/compare/v0.4.0...v0.5.0) - 2026-10-14

### Added

- *(broker)* sign sandboxed Agents' commits through the Broker

## [0.4.0](https://github.com/ajms/orchestrator/compare/v0.3.0...v0.4.0) - 2026-10-14

### Added

- *(tui)* show Broker refusals and failures in yellow
- *(broker)* let sandboxed Agents write issues in their Repo

### Fixed

- *(broker)* report a push that reached origin as pushed
- *(broker)* quiet gh auth and move Broker notices out of the pane

## [0.3.0](https://github.com/ajms/orchestrator/compare/v0.2.0...v0.3.0) - 2026-10-14

### Added

- *(broker)* gh repo view of this Repo for sandboxed Agents

### Other

- keep injected git config out of test repos

## [0.2.0](https://github.com/ajms/orchestrator/compare/v0.1.2...v0.2.0) - 2026-10-14

### Added

- *(sandbox)* [sandbox] config widens and tightens the Sandbox
- *(sandbox)* [**breaking**] the Sandbox is on by default
- *(broker)* gh for sandboxed Agents
- *(broker)* push for sandboxed Agents
- *(sandbox)* fail closed when the Sandbox can't run
- *(sandbox)* a Preset runs Claude in bubblewrap

### Fixed

- *(sandbox)* close review gaps in [sandbox] config

### Other

- *(store)* the downgrade test knows push and pull_request

## [0.1.2](https://github.com/ajms/orchestrator/compare/v0.1.1...v0.1.2) - 2026-10-13

### Fixed

- a rollback from the Sandbox release keeps working

## [0.1.1](https://github.com/ajms/orchestrator/compare/v0.1.0...v0.1.1) - 2026-10-13

### Other

- *(holder)* let the alternate-screen mirror catch up with the Holder before comparing
- *(daemon)* match the worktree-hook launch line across a resize's line breaks

## [0.1.0](https://github.com/ajms/orchestrator/releases/tag/v0.1.0) - 2026-10-13

### Added

- *(agent)* Antigravity Guards and Preset rule enforcement
- *(agent)* Antigravity Subagents and transcripts
- *(agent)* Antigravity Drafts from a fresh agy -p with the Base diff
- *(agent)* Antigravity usage samples and quota windows
- *(agent)* Antigravity launch, resume, Agent states and mode tracking
- *(agent)* orch agent install/uninstall antigravity
- Agent-neutral modes and per-Agent Preset rules
- route hook and tap payloads by --agent
- *(usage)* named usage windows per Agent
- store and choose a Session's Agent
- *(guards)* Agent-neutral Guarded actions and the ExternalTool Guard
- show the Subagent description in sidebar rows
- *(agent)* give Subagent worktrees a home inside the Session
- follow and stream Subagent transcripts
- *(tui)* pick the Repo with fuzzy matching and folder browsing
- show the Session title given through the Agent
- *(release)* tag and publish every green merge to main
- *(holder)* relay Agent OSC 52 copies to the Clients showing the Session
- *(daemon)* display variables from the latest Client reach Agents
- *(tui)* copy to clipboard and PRIMARY via wl-copy, xclip or OSC 52

### Fixed

- *(cli)* answer an untagged agy tool call that is not a Guard with the fallback decision
- *(agent)* flag an Antigravity hook installed without --event tags as Broken
- *(agent)* keep an Antigravity Session Starting on agy's trust screen
- *(agent)* run Antigravity Drafts in agy's stream-json print mode
- *(agent)* close Antigravity Preset rule bypasses
- *(agent)* attribute adopted agy Subagents and keep their hooks off the Session's turn
- *(daemon)* cap the Draft's Base diff and keep Claude's Draft fallback
- *(agent)* tighten Antigravity Agent states after review
- *(agent)* make the Antigravity hookup fail safe and restore exactly
- *(usage)* tidy usage windows after review
- tidy --agent routing after review
- keep finished sessions Idle until a prompt is submitted
- *(daemon)* follow Subagents in the order a Client asks
- keep git redirect variables away from spawned processes and fixtures

### Other

- *(release)* version with SemVer via release-plz
- Revert "add the Antigravity Agent adapter alongside Claude" ([#95](https://github.com/ajms/orchestrator/pull/95))
- *(daemon)* match the worktree-hook launch line across a resize's line breaks
- *(holder)* let the alternate-screen mirror catch up with the Holder before comparing
- *(agent)* give agy's final Stop more time and list the Stops seen when it is missing
- *(agent)* tidy the fake agy Draft and assert only the meaningful Draft argv
- *(agent)* let adapters own their Draft framing behind DraftOutcome
- *(agent)* aim the real-agy Guard check outside the temp dirs and wait for the Stop capture
- *(agent)* accept agy 1.3's off-machine data-sharing choice and retry stuck onboarding screens
- *(agent)* dedupe built-in adapter builders and narrow tag_hook_event
- *(agent)* keep real-agy recordings per Session and agy fully isolated
- *(agent)* opt-in real-agy suite with a template HOME and fixture re-recording
- tidy Session Agent code after review
- drop production seams only tests called
- CLI wiring, doctor, repo move/forget, trust, systemd unit, docs
- Notifications, usage report and rate limits in the Daemon
- Reconciliation
- Landing, Discard, PR flow, stacking, :preset and :mute
- Daemon and Client protocol
- Holder, hook and statusline reporting, fake Agent, key encoding
- Core domain: event vocabulary, Phase and Agent state machine, flags, gates
