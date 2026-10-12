use orch_agent::{
    AgentAdapter, Antigravity, DraftInput, DraftOutcome, LaunchSpec, Preset, Presets,
};
use orch_core::{ConversationId, PermissionMode, SessionId};
use serde_json::{Value, json};

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";

fn preset(name: &str) -> Preset {
    Presets::default().get(name).unwrap().clone()
}

fn spec(preset: Preset) -> LaunchSpec {
    LaunchSpec::new(SessionId("fix-login".into()), "/opt/orch/bin/orch", preset)
}

fn launched(preset: Preset) -> Vec<String> {
    Antigravity::default()
        .launch(&spec(preset).with_prompt("Fix the login bug"))
        .args
}

fn resumed(preset: Preset, observed: Option<PermissionMode>) -> Vec<String> {
    Antigravity::default()
        .resume(
            &spec(preset),
            &ConversationId(CONVERSATION.into()),
            observed,
        )
        .expect("agy resumes")
        .args
}

#[test]
fn launch_runs_agy_with_the_prompt_submitted_interactively() {
    let argv = Antigravity::default().launch(&spec(preset("ask")).with_prompt("Fix the login bug"));
    assert_eq!(argv.program, "agy");
    assert_eq!(argv.args, ["-i", "Fix the login bug"]);
}

#[test]
fn launch_without_a_prompt_just_opens_agy() {
    let argv = Antigravity::default().launch(&spec(preset("ask")));
    assert!(argv.args.is_empty(), "{:?}", argv.args);
}

#[test]
fn launch_sets_agys_mode_from_the_preset() {
    assert_eq!(
        launched(preset("edits")),
        ["--mode", "accept-edits", "-i", "Fix the login bug"]
    );
    assert_eq!(
        launched(preset("plan")),
        ["--mode", "plan", "-i", "Fix the login bug"]
    );
    assert_eq!(launched(Preset::inherit()), ["-i", "Fix the login bug"]);
}

#[test]
fn resume_continues_the_conversation_in_the_last_observed_mode() {
    assert_eq!(
        resumed(preset("edits"), Some(PermissionMode::Plan)),
        ["--conversation", CONVERSATION, "--mode", "plan"]
    );
    assert_eq!(
        resumed(preset("edits"), Some(PermissionMode::Default)),
        ["--conversation", CONVERSATION]
    );
}

#[test]
fn resume_falls_back_to_the_presets_mode_before_any_was_observed() {
    assert_eq!(
        resumed(preset("edits"), None),
        ["--conversation", CONVERSATION, "--mode", "accept-edits"]
    );
}

#[test]
fn an_inherit_session_resumes_in_whatever_mode_agy_defaults_to() {
    assert_eq!(
        resumed(Preset::inherit(), Some(PermissionMode::Plan)),
        ["--conversation", CONVERSATION]
    );
}

#[test]
fn antigravity_offers_only_the_modes_agy_can_launch() {
    let presets = Presets::default();
    let offered: Vec<&str> = presets
        .offered(Antigravity::default().modes())
        .map(|preset| preset.name.as_str())
        .collect();
    assert_eq!(offered, ["plan", "ask", "edits", "inherit"]);
}

#[test]
fn draft_runs_a_fresh_headless_agy_in_stream_json_fed_the_base_diff_whatever_the_conversation() {
    let conversation = ConversationId(CONVERSATION.into());
    for latest in [Some(&conversation), None] {
        let draft = Antigravity::default().draft(latest).expect("agy drafts");
        assert_eq!(draft.argv.program, "agy");
        assert_eq!(
            draft.argv.args,
            [
                "-p",
                "",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json"
            ]
        );
        assert_eq!(draft.input, DraftInput::InstructionAndBaseDiff);
    }
}

#[test]
fn agys_draft_prompt_is_one_user_event_line() {
    let prompt = "Write a commit message.\n\ndiff --git a/x b/x\n+\"quoted\"";
    let encoded = Antigravity::default().encode_draft(prompt);
    assert!(
        encoded.ends_with('\n') && encoded.lines().count() == 1,
        "{encoded:?}"
    );
    let line: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        line,
        json!({ "event": "user", "message": { "content": [{ "type": "text", "text": prompt }] } })
    );
}

fn agy_output(result: Value) -> String {
    let events = [
        json!({ "event": "init", "conversation_id": CONVERSATION }),
        json!({ "event": "step_update", "step": { "type": "PLANNER_RESPONSE" } }),
        json!({ "event": "result", "result": result }),
    ];
    events.map(|event| format!("{event}\n")).concat()
}

#[test]
fn agys_draft_is_the_response_of_its_successful_result_event() {
    let output = agy_output(json!({
        "conversation_id": CONVERSATION,
        "status": "SUCCESS",
        "response": "Fix the login bug\n\nCheck the password hash.\n",
    }));
    assert_eq!(
        Antigravity::default().decode_draft(&output),
        DraftOutcome::Drafted("Fix the login bug\n\nCheck the password hash.\n".into())
    );
}

#[test]
fn an_agy_error_result_fails_the_draft_with_agys_error() {
    let output = agy_output(json!({
        "conversation_id": CONVERSATION,
        "status": "ERROR",
        "error": "quota exhausted for gemini-weekly",
    }));
    let failed = Antigravity::default().decode_draft(&output);
    assert!(
        matches!(&failed, DraftOutcome::Failed(error) if error.contains("quota exhausted for gemini-weekly")),
        "{failed:?}"
    );
}

#[test]
fn agy_output_without_a_result_event_has_no_draft() {
    let output = format!(
        "{}\n",
        json!({ "event": "init", "conversation_id": CONVERSATION })
    );
    assert_eq!(
        Antigravity::default().decode_draft(&output),
        DraftOutcome::NoResult
    );
}

#[test]
fn a_successful_agy_result_without_a_response_text_fails_as_an_empty_response() {
    for response in [None, Some(json!(42)), Some(json!(" \n"))] {
        let mut result = json!({ "conversation_id": CONVERSATION, "status": "SUCCESS" });
        if let Some(response) = &response {
            result["response"] = response.clone();
        }
        assert_eq!(
            Antigravity::default().decode_draft(&agy_output(result)),
            DraftOutcome::Failed("agy returned an empty response".into()),
            "response {response:?}"
        );
    }
}
