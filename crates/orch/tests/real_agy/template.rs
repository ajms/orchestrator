use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use orch_holder::{HolderClient, ToHolder};
use serde_json::{Value, json};

use crate::agy::{Held, STARTUP, TRUST_ANSWER, hold, is_trust_screen, isolate};

pub const CAPTURE_STATUSLINE: &str = r#"if [ -n "$ORCH_REAL_AGY_CAPTURE" ]; then cat > "$ORCH_REAL_AGY_CAPTURE/statusline-$(date +%s%N).json"; else cat > /dev/null; fi"#;
const ONBOARDED: &str = "onboarded";
const STAMP: &str = "stamp";
const DOWN: &[u8] = b"\x1b[B";
const ENTER: &[u8] = b"\r";

static BUILDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("real-agy-template")
}

pub async fn prepare(agy: &Path) -> PathBuf {
    let _building = BUILDING.lock().await;
    let dir = dir();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let stamp = stamp(&dir, agy);
    match std::fs::read_to_string(dir.join(STAMP)) {
        Ok(found) if found == stamp => return dir,
        Ok(_) => std::fs::remove_dir_all(&dir).unwrap(),
        Err(_) => {}
    }
    for sub in ["home", "state", "config", "work"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    if !dir.join(ONBOARDED).exists() {
        onboard(&dir, agy).await;
        std::fs::write(dir.join(ONBOARDED), "").unwrap();
    }
    let settings = dir.join("home/.gemini/antigravity-cli/settings.json");
    refuse_data_sharing(&settings);
    set_statusline(&settings);
    install_hookup(&dir);
    std::fs::write(dir.join(STAMP), stamp).unwrap();
    dir
}

fn stamp(dir: &Path, agy: &Path) -> String {
    let version = template_command(dir, agy)
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

fn template_command(dir: &Path, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    isolate(&mut command, &dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"));
    command
}

fn manual(dir: &Path, screen: &str) -> String {
    format!(
        "agy's onboarding could not be driven. Screen:\n{screen}\n\
         Onboard the template by hand: from {work} run\n  \
         HOME={home} XDG_CONFIG_HOME={config} XDG_STATE_HOME={state} XDG_DATA_HOME={home}/.local/share XDG_CACHE_HOME={home}/.cache agy\n\
         pick any theme, turn data sharing OFF, leave agy at the trust screen with ctrl+c, then\n  \
         touch {marker}\nand run the suite again.",
        work = dir.join("work").display(),
        home = dir.join("home").display(),
        config = dir.join("config").display(),
        state = dir.join("state").display(),
        marker = dir.join(ONBOARDED).display(),
    )
}

async fn onboard(dir: &Path, agy: &Path) {
    let runtime = tempfile::Builder::new().prefix("oa").tempdir().unwrap();
    let mut held = hold(
        template_command(dir, env!("CARGO_BIN_EXE_orch")),
        runtime.path(),
        "onboarding",
        &dir.join("work"),
        agy,
    )
    .await;
    drive_onboarding(&mut held, dir).await;
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

async fn drive_onboarding(held: &mut Held, dir: &Path) {
    let deadline = Instant::now() + 2 * STARTUP;
    let mut consented = false;
    let mut acted_on = String::new();
    let mut last = String::new();
    let mut changed_at = Instant::now();
    loop {
        let text = screen(&mut held.client).await;
        assert!(Instant::now() < deadline, "{}", manual(dir, &text));
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
                choose_off(&mut held.client, dir).await;
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
    const MARKERS: [char; 7] = ['❯', '›', '▸', '►', '>', '●', '◉'];
    text.lines()
        .find(|line| line.trim_start().starts_with(MARKERS))
}

fn is_off(line: &str) -> bool {
    const OFF: [&str; 9] = [
        "no", "off", "disable", "disabled", "decline", "don", "not", "never", "deny",
    ];
    line.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| OFF.contains(&word))
}

async fn choose_off(client: &mut HolderClient, dir: &Path) {
    for _ in 0..8 {
        let text = screen(client).await;
        match selected_line(&text) {
            Some(line) if is_off(line) => {
                press(client, ENTER).await;
                return;
            }
            Some(_) => press(client, DOWN).await,
            None => panic!("{}", manual(dir, &text)),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("{}", manual(dir, &screen(client).await));
}

fn sharing_keys(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let path = format!("{path}.{key}");
                let key = key.to_lowercase();
                let sharing = ["shar", "telemetry", "improve", "usagestat"]
                    .iter()
                    .any(|needle| key.contains(needle));
                match value {
                    Value::Bool(true) if sharing => found.push(path),
                    _ => sharing_keys(value, &path, found),
                }
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| sharing_keys(item, path, found)),
        _ => {}
    }
}

fn read_settings(settings: &Path) -> Value {
    std::fs::read_to_string(settings)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn refuse_data_sharing(settings: &Path) {
    let mut on = Vec::new();
    sharing_keys(&read_settings(settings), "", &mut on);
    assert!(
        on.is_empty(),
        "data sharing looks on in {}: {on:?}; delete {} and run again",
        settings.display(),
        dir().display()
    );
}

fn set_statusline(settings: &Path) {
    let mut value = read_settings(settings);
    value["statusLine"] = json!({ "type": "command", "command": CAPTURE_STATUSLINE });
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(settings, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn install_hookup(dir: &Path) {
    let installed = template_command(dir, env!("CARGO_BIN_EXE_orch"))
        .args(["agent", "install", "antigravity", "--yes"])
        .current_dir(dir.join("work"))
        .output()
        .unwrap();
    assert!(installed.status.success(), "{installed:?}");
}
