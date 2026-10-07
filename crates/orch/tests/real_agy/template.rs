use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use orch_holder::{HolderClient, ToHolder};
use serde_json::{Value, json};

use crate::agy::{Held, STARTUP, TRUST_ANSWER, hold, is_trust_screen};
use crate::isolation::Isolation;

pub const CAPTURE_STATUSLINE: &str = r#"d="$ORCH_REAL_AGY_CAPTURE/$ORCH_SESSION"; if [ -n "$ORCH_REAL_AGY_CAPTURE" ] && [ -n "$ORCH_SESSION" ]; then mkdir -p "$d" && cat > "$d/statusline-$(date +%s%N).json"; else cat > /dev/null; fi"#;
pub const SHARING_KEYS: [&str; 5] = [
    "telemetryEnabled",
    "dataSharingEnabled",
    "dataSharing",
    "shareUsageData",
    "usageStatisticsEnabled",
];
const ONBOARDED: &str = "onboarded";
const STAMP: &str = "stamp";
const DOWN: &[u8] = b"\x1b[B";
const ENTER: &[u8] = b"\r";

pub fn template_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("real-agy-template")
}

pub async fn prepare(agy: &Path) -> PathBuf {
    let dir = template_dir();
    let isolation = Isolation::under(&dir);
    std::fs::create_dir_all(&isolation.home).unwrap();
    let stamp = stamp(&isolation, agy);
    match std::fs::read_to_string(dir.join(STAMP)) {
        Ok(found) if found == stamp => return dir,
        Ok(_) => std::fs::remove_dir_all(&dir).unwrap(),
        Err(_) => {}
    }
    for sub in ["home", "state", "config", "work"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    if !dir.join(ONBOARDED).exists() {
        onboard(&dir, &isolation, agy).await;
        std::fs::write(dir.join(ONBOARDED), "").unwrap();
    }
    let settings = isolation.home.join(".gemini/antigravity-cli/settings.json");
    let verdict = sharing_verdict(std::fs::read_to_string(&settings).ok().as_deref());
    if let Err(problem) = verdict {
        panic!(
            "{}: {problem}\n{}",
            settings.display(),
            manual(&dir, &isolation, agy, "")
        );
    }
    set_statusline(&settings);
    install_hookup(&dir, &isolation);
    std::fs::write(dir.join(STAMP), stamp).unwrap();
    dir
}

fn stamp(isolation: &Isolation, agy: &Path) -> String {
    let mut command = Command::new(agy);
    let version = isolation
        .apply(&mut command)
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success(), "{agy:?} --version: {version:?}");
    format!(
        "{}{}\n",
        String::from_utf8_lossy(&version.stdout),
        env!("CARGO_BIN_EXE_orch")
    )
}

fn manual(dir: &Path, isolation: &Isolation, agy: &Path, screen: &str) -> String {
    format!(
        "agy's onboarding needs a hand. Screen:\n{screen}\n\
         Delete {dir} if it holds a bad onboarding, then from {work} run\n  {line}\n\
         pick any theme, turn data sharing OFF, quit agy at the trust screen with ctrl+c, then\n  \
         touch {marker}\nand run the suite again. If agy's data-sharing setting is not one of {SHARING_KEYS:?}, add its key to SHARING_KEYS in {file}.",
        dir = dir.display(),
        work = dir.join("work").display(),
        line = isolation.shell_line(agy),
        marker = dir.join(ONBOARDED).display(),
        file = file!(),
    )
}

async fn onboard(dir: &Path, isolation: &Isolation, agy: &Path) {
    let runtime = tempfile::Builder::new().prefix("oa").tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_orch"));
    isolation.apply(&mut command);
    let argv: Vec<OsString> = vec![agy.into()];
    let mut held = hold(
        command,
        runtime.path(),
        "onboarding",
        &dir.join("work"),
        &argv,
    )
    .await;
    let manual = |screen: &str| manual(dir, isolation, agy, screen);
    drive_onboarding(&mut held, &manual).await;
    held.client.send(&ToHolder::Kill).await.unwrap();
}

async fn screen(client: &mut HolderClient) -> String {
    client.snapshot().await.unwrap().text()
}

async fn press(client: &mut HolderClient, bytes: &[u8]) {
    let input = ToHolder::Input {
        bytes: bytes.to_vec(),
    };
    client.send(&input).await.unwrap();
}

fn is_consent_screen(lower: &str) -> bool {
    ["shar", "telemetry", "usage statistics", "improve"]
        .iter()
        .any(|needle| lower.contains(needle))
}

async fn drive_onboarding(held: &mut Held, manual: &dyn Fn(&str) -> String) {
    let deadline = Instant::now() + 2 * STARTUP;
    let mut consented = false;
    let mut acted_on = String::new();
    let mut last = String::new();
    let mut changed_at = Instant::now();
    loop {
        let text = screen(&mut held.client).await;
        assert!(Instant::now() < deadline, "{}", manual(&text));
        if text != last {
            last = text.clone();
            changed_at = Instant::now();
        }
        let lower = text.to_lowercase();
        let settled = changed_at.elapsed() > Duration::from_secs(3);
        if text != acted_on {
            if is_trust_screen(&lower) {
                if consented {
                    return;
                }
                press(&mut held.client, TRUST_ANSWER).await;
                acted_on = text;
            } else if is_consent_screen(&lower) {
                choose_off(&mut held.client, manual).await;
                consented = true;
                acted_on = text;
            } else if lower.contains("theme") {
                press(&mut held.client, ENTER).await;
                acted_on = text;
            } else if consented && settled {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn selected_line(text: &str) -> Option<&str> {
    const MARKERS: [char; 6] = ['❯', '›', '▸', '►', '●', '◉'];
    let marked = |line: &&str| line.trim_start().starts_with(MARKERS);
    let quoted = |line: &&str| {
        let line = line.trim_start();
        line.starts_with("> ") && !line[2..].trim().is_empty()
    };
    text.lines()
        .find(marked)
        .or_else(|| text.lines().find(quoted))
}

fn is_off(line: &str) -> bool {
    const OFF: [&str; 9] = [
        "no", "off", "disable", "disabled", "decline", "don", "not", "never", "deny",
    ];
    line.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| OFF.contains(&word))
}

async fn choose_off(client: &mut HolderClient, manual: &dyn Fn(&str) -> String) {
    for _ in 0..8 {
        let text = screen(client).await;
        match selected_line(&text) {
            Some(line) if is_off(line) => {
                press(client, ENTER).await;
                return;
            }
            Some(_) => press(client, DOWN).await,
            None => panic!("{}", manual(&text)),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("{}", manual(&screen(client).await));
}

fn walk<'a>(value: &'a Value, path: &str, found: &mut Vec<(String, &'a Value)>) {
    match value {
        Value::Object(map) => map.iter().for_each(|(key, value)| {
            let path = format!("{path}.{key}");
            found.push((path.clone(), value));
            walk(value, &path, found);
        }),
        Value::Array(items) => items.iter().for_each(|item| walk(item, path, found)),
        _ => {}
    }
}

pub fn sharing_verdict(settings: Option<&str>) -> Result<Vec<String>, String> {
    let settings = settings.ok_or("agy wrote no settings.json")?;
    let value: Value = serde_json::from_str(settings)
        .ok()
        .filter(Value::is_object)
        .ok_or("settings.json is not a JSON object")?;
    let mut entries = Vec::new();
    walk(&value, "", &mut entries);
    let key = |path: &str| path.rsplit('.').next().unwrap_or_default().to_string();
    let sharing_on: Vec<String> = entries
        .iter()
        .filter(|(path, value)| {
            let key = key(path).to_lowercase();
            **value == true
                && ["shar", "telemetry", "improve", "usagestat"]
                    .iter()
                    .any(|needle| key.contains(needle))
        })
        .map(|(path, _)| path.clone())
        .collect();
    if !sharing_on.is_empty() {
        return Err(format!("data sharing is on: {sharing_on:?}"));
    }
    let off: Vec<String> = entries
        .iter()
        .filter(|(path, value)| **value == false && SHARING_KEYS.contains(&key(path).as_str()))
        .map(|(path, _)| path.clone())
        .collect();
    match off.is_empty() {
        true => Err(format!(
            "none of {SHARING_KEYS:?} is set to false, so data sharing is not known to be off"
        )),
        false => Ok(off),
    }
}

fn set_statusline(settings: &Path) {
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
    value["statusLine"] = json!({ "type": "command", "command": CAPTURE_STATUSLINE });
    std::fs::write(settings, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn install_hookup(dir: &Path, isolation: &Isolation) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orch"));
    let installed = isolation
        .apply(&mut command)
        .args(["agent", "install", "antigravity", "--yes"])
        .current_dir(dir.join("work"))
        .output()
        .unwrap();
    assert!(installed.status.success(), "{installed:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_option_is_the_line_with_a_selection_marker() {
        let screen =
            "Help improve agy?\n> Quoted note\n  Yes, share usage data\n ❯ No, keep it off\n";

        assert_eq!(selected_line(screen), Some(" ❯ No, keep it off"));
        assert_eq!(selected_line("  >\n  plain\n"), None);
    }

    #[test]
    fn an_option_reads_as_off_by_its_words_not_their_letters() {
        assert!(is_off("❯ No, don't share"));
        assert!(is_off("› Turn data sharing off"));
        assert!(!is_off("❯ Yes, share now and help us know more"));
    }

    #[test]
    fn data_sharing_counts_as_off_only_when_a_known_key_says_so() {
        assert_eq!(
            sharing_verdict(Some(r#"{"telemetryEnabled":false,"theme":"dark"}"#)),
            Ok(vec![".telemetryEnabled".to_string()])
        );
        assert!(sharing_verdict(None).is_err());
        assert!(sharing_verdict(Some("not json")).is_err());
        assert!(sharing_verdict(Some(r#"{"theme":"dark"}"#)).is_err());
        assert!(
            sharing_verdict(Some(
                r#"{"telemetryEnabled":false,"ux":{"shareCrashes":true}}"#
            ))
            .is_err()
        );
    }
}
