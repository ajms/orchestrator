use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use orch_protocol::{Reply, RepoSettings, Request};

use crate::client::{self, ClientError};
use crate::repo::absolute;

#[derive(Args)]
pub struct TrustArgs {
    repo: PathBuf,
    #[arg(long)]
    yes: bool,
}

const DECLINED: u8 = 1;
const FAILED: u8 = 2;

pub fn run(args: TrustArgs) -> ExitCode {
    match trust(args) {
        Ok(code) => code,
        Err(err) => client::fail("trust", FAILED, err),
    }
}

fn trust(args: TrustArgs) -> Result<ExitCode, ClientError> {
    let request = Request::RepoSettings {
        repo: absolute(&args.repo),
    };
    let settings: RepoSettings = client::expect(request, |reply| match reply {
        Reply::RepoSettings(settings) => Some(settings),
        _ => None,
    })?;
    let Some(needed) = settings.trust else {
        println!(
            "{}: the Repo's config needs no Trust.",
            settings.repo.display()
        );
        return Ok(ExitCode::SUCCESS);
    };
    println!(
        "{} brings scripts or Presets that need your Trust:",
        settings.repo.display()
    );
    for item in &needed.items {
        println!("  {item}");
    }
    if !args.yes && !confirmed() {
        println!("Not trusted.");
        return Ok(ExitCode::from(DECLINED));
    }
    client::request(Request::ApproveTrust {
        repo: settings.repo,
        hash: needed.hash,
    })?;
    println!("Trusted.");
    Ok(ExitCode::SUCCESS)
}

fn confirmed() -> bool {
    print!("Trust these? [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes")
}
