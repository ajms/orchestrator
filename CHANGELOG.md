# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.18.0](https://github.com/ajms/orchestrator/compare/v0.17.0...v0.18.0) - 2026-11-10

### Added

- *(subagents)* report a conflicted or abandoned merge-back
- *(pi)* route pi subagent worktree hooks through orch worktree-hook
- *(subagents)* show how a Subagent ended ([#239](https://github.com/ajms/orchestrator/pull/239))
- *(subagents)* record base/head diff refs for subagent worktrees
- *(tui)* review a single Subagent's worktree with d/D

### Fixed

- *(tui)* carry Subagent name with Review results instead of App state
- *(tui)* size the Subagent outcome rule to the pane width
- *(tui)* show the Subagent's branch under its Review's file list

### Other

- *(subagents)* record the Subagent tip loudly and review via a validated ReviewScope
- *(subagents)* address review of #243
- *(tui)* send the Subagent name with the external Review result

## [0.17.0](https://github.com/ajms/orchestrator/compare/v0.16.1...v0.17.0) - 2026-11-09

### Added

- *(broker)* let sandboxed Agents post whole PR reviews

### Fixed

- *(broker)* refuse a whole review that lost its input

## [0.16.1](https://github.com/ajms/orchestrator/compare/v0.16.0...v0.16.1) - 2026-11-07

### Other

- *(sandbox)* set up the remote before the Session starts

## [0.16.0](https://github.com/ajms/orchestrator/compare/v0.15.1...v0.16.0) - 2026-11-07

### Added

- *(claude)* project the Agent profile into a Sandbox tmpfs
- *(broker)* allow pr comment, pr review, and inline review comments
- *(claude)* run from a scoped CLAUDE_CONFIG_DIR profile

### Fixed

- *(broker)* harden PR review calls and guard approvals
- *(claude)* bind carve-outs at their real target so the sandbox launches
- *(git)* keep the index mtime when snapshotting a Worktree

### Other

- Merge pull request #230 from ajms/orch/plan-lets-analyse-how-we-can
- *(config)* update hard-wall paths to the Claude profile carve-outs

## [0.15.1](https://github.com/ajms/orchestrator/compare/v0.15.0...v0.15.1) - 2026-11-06

### Fixed

- *(tui)* bound tail rendering and cover the Preparing pane
- *(tui)* keep the end of long Setup output in view

## [0.15.0](https://github.com/ajms/orchestrator/compare/v0.14.1...v0.15.0) - 2026-11-04

### Added

- *(broker)* let sandboxed Agents read repos of readable owners

## [0.14.1](https://github.com/ajms/orchestrator/compare/v0.14.0...v0.14.1) - 2026-11-03

### Fixed

- *(agent)* render pi's mcp.json with the Agent's env instead of linking it

## [0.14.0](https://github.com/ajms/orchestrator/compare/v0.13.0...v0.14.0) - 2026-11-03

### Added

- *(agent)* link the user's pi mcp.json into the Agent profile

## [0.13.0](https://github.com/ajms/orchestrator/compare/v0.12.0...v0.13.0) - 2026-11-02

### Added

- *(agent)* give an Agent extra variables from personal commands

## [0.12.0](https://github.com/ajms/orchestrator/compare/v0.11.0...v0.12.0) - 2026-11-02

### Added

- *(agent)* append an Under orch section to pi's system prompt

### Fixed

- *(agent)* link the user's skills into pi's Agent profile

## [0.11.0](https://github.com/ajms/orchestrator/compare/v0.10.0...v0.11.0) - 2026-11-02

### Added

- pick a Session's Agent in the :new form

## [0.10.0](https://github.com/ajms/orchestrator/compare/v0.9.0...v0.10.0) - 2026-11-02

### Added

- *(agent)* pi reports Agent states, Conversations and Session titles
- *(daemon)* launch pi from its Agent profile and resume its Conversation
- store each Session's Agent and route hooks by Agent
- pi drafts from a fork of its Conversation and the Base diff
- *(agent)* pi reports Subagents and follows their transcripts
- *(agent)* a pi subagent call is a Subagent row
- *(agent)* orch's pi extension writes each subagent call's transcript
- *(agent)* pi reports usage
- *(agent)* pi's extension taps orch with its Conversation's usage
- *(agent)* orch's pi extension forwards pi's coarse events to orch hook
- *(agent)* build pi's Agent profile from the user's own setup
- *(agent)* a pi adapter that launches and resumes by session id
- *(tui)* show each Session's Agent and the Agent :new will use

### Fixed

- *(daemon)* cap the Base diff a Draft reads at 256 KiB
- *(daemon)* a Draft's time limit covers writing its stdin
- *(daemon)* adopt an orphaned Holder under the Agent it runs
- *(agent)* pi's extension caps a Subagent id in bytes, as orch reads it
- *(agent)* a pi Subagent row without agent or task name still has labels
- *(agent)* never read a pi Subagent transcript that is hardlinked elsewhere
- *(agent)* map pi's tap to Claude's token units
- *(agent)* pi's tap carries the last response's tokens, like Claude's
- *(agent)* launch pi from the Agent profile prepare builds
- *(agent)* pi's extension sends only the fields map_hook reads
- *(agent)* read a pi error's status code only where it is one
- *(agent)* rebuild only what orch generates in pi's Agent profile
- *(agent)* never follow an Agent's links when rebuilding pi's profile
- *(daemon)* store a launch's named Conversation before the Holder starts
- *(tui)* tag each Session row with its Agent on the first line

### Other

- only a Draft that needs a scratch dir gets one
- *(daemon)* a pi Draft forks, reads the Base diff and leaves nothing behind
- *(daemon)* a pi subagent call is a Subagent row with a live transcript
- *(daemon)* a repeated pi tap adds nothing to the Repo's usage
- *(daemon)* pi taps add to the Repo's usage, not the rate limits
- *(daemon)* an opt-in smoke test runs real pi
- *(daemon)* a pi Session resumes after a suspend or an exit, and its profile is reset
- *(daemon)* cover a dropped Guard and a lost binary of a Session's Agent
- *(protocol)* name a Session's Agent `agent`, its state `agent_state`
- look up every Agent adapter by name in one registry
- say what pi's Subagent support assumes, and list its id rules
- *(agent)* path is the last key argument for every Agent
- *(agent)* key pi's Subagent transcripts by SubagentId and ConversationId
- *(agent)* one hardened line follower for Claude's and pi's transcripts
- *(agent)* name the helper that finds a pi subagent call's Subagent
- *(agent)* subagent_transcripts takes the env lookup, like agent_profile
- pi usage in Claude's token units, and when pi taps
- *(agent)* pin pi's siblings so the pinned pi 1.1.0 runs
- *(agent)* tidy pi's hook payload types and extension files
- *(agent)* record pi's hook payloads from real pi
- *(agent)* resolve pi's dirs once and tidy the registry
- one default for an Agent that isn't named
- *(daemon)* record a Conversation through one helper

## [0.9.0](https://github.com/ajms/orchestrator/compare/v0.8.0...v0.9.0) - 2026-10-28

### Added

- *(tui)* show what the :new Branch field names before creating
- *(daemon)* start a Session from a PR and follow PR retargets
- *(daemon)* start a Session on an Existing Branch

## [0.8.0](https://github.com/ajms/orchestrator/compare/v0.7.0...v0.8.0) - 2026-10-26

### Added

- *(sandbox)* clear the Agent's environment with --clearenv

## [0.7.0](https://github.com/ajms/orchestrator/compare/v0.6.0...v0.7.0) - 2026-10-26

### Added

- *(git)* refuse a new Session when the Base can't match origin

## [0.6.0](https://github.com/ajms/orchestrator/compare/v0.5.2...v0.6.0) - 2026-10-24

### Added

- *(broker)* route sandboxed Agents' fetches through the Broker
- *(tui)* show :sh and :! shells in the Session pane
- *(tui)* run commands outside the Sandbox with :! and :sh

### Fixed

- *(tui)* close ended shells on a full Session list
- *(tui)* keep typing in the shell and close it when its Session ends

## [0.5.2](https://github.com/ajms/orchestrator/compare/v0.5.1...v0.5.2) - 2026-10-15

### Fixed

- *(broker)* route a repo's own push URLs through the Broker
- *(broker)* scope the push rewrite to the Session repo's origin

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
