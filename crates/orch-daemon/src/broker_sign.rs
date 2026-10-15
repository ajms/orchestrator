use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_git::ENV_REDIRECTING_GIT;

pub(crate) const IDENTITY_FILE: &str = "agent-identity";
const LIMIT: Duration = Duration::from_secs(10);

pub(crate) struct Identity {
    pub(crate) name: String,
    pub(crate) email: String,
}

impl Identity {
    pub(crate) fn line(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }
}

pub(crate) fn user_identity(repo: &Path) -> Result<Option<Identity>, String> {
    let mut ident = std::process::Command::new("git");
    for key in ENV_REDIRECTING_GIT {
        ident.env_remove(key);
    }
    let output = ident
        .arg("-C")
        .arg(repo)
        .args(["var", "GIT_COMMITTER_IDENT"])
        .output()
        .map_err(|err| format!("running git var: {err}"))?;
    Ok(agent_identity(&String::from_utf8_lossy(&output.stdout)))
}

pub(crate) fn origin_url(repo: &Path) -> Result<Option<String>, String> {
    let mut config = std::process::Command::new("git");
    for key in ENV_REDIRECTING_GIT {
        config.env_remove(key);
    }
    let output = config
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .map_err(|err| format!("running git config: {err}"))?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!url.is_empty()).then_some(url))
}

pub(crate) fn public_key(key: &Path) -> Result<String, String> {
    let mut public = key.as_os_str().to_owned();
    public.push(".pub");
    let public = PathBuf::from(public);
    std::fs::read_to_string(&public)
        .map(|text| text.trim().to_string())
        .map_err(|err| format!("reading the Agent signing key {}: {err}", public.display()))
}

pub(crate) async fn sign(
    agent: Option<String>,
    key: Option<PathBuf>,
    payload: Vec<u8>,
) -> Result<String, String> {
    let key = key.ok_or("no Agent signing key is configured")?;
    let agent = agent.ok_or("the Agent has no identity to sign as")?;
    allowed_signing(&payload, &agent)?;
    let mut signer = crate::subprocess::command("ssh-keygen");
    signer.args(["-Y", "sign", "-n", "git", "-f"]).arg(&key);
    crate::subprocess::run(signer, Some(payload), LIMIT, "signing").await
}

fn agent_identity(ident: &str) -> Option<Identity> {
    let (name, rest) = ident.split_once(" <")?;
    let (email, _) = rest.split_once('>')?;
    Some(Identity {
        name: format!("{} (orch)", name.trim()),
        email: email.into(),
    })
}

fn allowed_signing(payload: &[u8], agent: &str) -> Result<(), String> {
    let headers: Vec<&[u8]> = payload
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
        .collect();
    if !headers
        .first()
        .is_some_and(|line| line.starts_with(b"tree "))
    {
        return Err("the Broker signs only commits".into());
    }
    let committers: Vec<&[u8]> = headers
        .iter()
        .filter_map(|line| line.strip_prefix(b"committer ".as_slice()))
        .collect();
    let expected = format!("{agent} ");
    match committers.as_slice() {
        [committer] if committer.starts_with(expected.as_bytes()) => Ok(()),
        _ => Err(format!(
            "the Broker signs only commits committed as {agent}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "Ada (orch) <ada@example.com>";

    fn commit(committer: &str) -> Vec<u8> {
        format!(
            "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
             parent c0ffee0000000000000000000000000000000000\n\
             author Ada (orch) <ada@example.com> 1700000000 +0100\n\
             committer {committer} 1700000000 +0100\n\
             \n\
             Add a feature\n"
        )
        .into_bytes()
    }

    #[test]
    fn the_agent_identity_is_the_users_name_marked_orch_with_the_users_email() {
        let ident = "Ada Lovelace <ada@example.com> 1700000000 +0100";
        let identity = agent_identity(ident).unwrap();
        assert_eq!(identity.line(), "Ada Lovelace (orch) <ada@example.com>");
        assert!(agent_identity("no email here").is_none());
    }

    #[test]
    fn a_commit_the_agent_identity_committed_is_signed() {
        assert_eq!(allowed_signing(&commit(AGENT), AGENT), Ok(()));
    }

    #[test]
    fn a_commit_message_that_is_not_utf8_is_still_signed() {
        let mut latin1 = commit(AGENT);
        latin1.extend(b"caf\xe9\n");
        assert_eq!(allowed_signing(&latin1, AGENT), Ok(()));
    }

    #[test]
    fn a_commit_by_anyone_else_is_refused() {
        for committer in [
            "Ada <ada@example.com>",
            "Ada (orch) <other@example.com>",
            "Mallory (orch) <ada@example.com>",
        ] {
            let refusal = allowed_signing(&commit(committer), AGENT).unwrap_err();
            assert!(refusal.contains(AGENT), "{refusal}");
        }
    }

    #[test]
    fn anything_but_a_commit_object_is_refused() {
        let tag = "object c0ffee0000000000000000000000000000000000\ntype commit\ntag v1\n\
                   tagger Ada (orch) <ada@example.com> 1700000000 +0100\n\nv1\n";
        for payload in [tag, "just some text\n", ""] {
            let refusal = allowed_signing(payload.as_bytes(), AGENT).unwrap_err();
            assert!(refusal.contains("only commits"), "{refusal}");
        }
    }

    #[test]
    fn a_committer_line_in_the_message_or_a_second_committer_does_not_count() {
        let in_message = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
                          author Ada (orch) <ada@example.com> 1700000000 +0100\n\
                          \n\
                          committer Ada (orch) <ada@example.com> 1700000000 +0100\n";
        assert!(allowed_signing(in_message.as_bytes(), AGENT).is_err());

        let twice = String::from_utf8(commit(AGENT)).unwrap().replacen(
            "\n\n",
            "\ncommitter Mallory <m@example.com> 1700000000 +0100\n\n",
            1,
        );
        assert!(allowed_signing(twice.as_bytes(), AGENT).is_err());
    }
}
