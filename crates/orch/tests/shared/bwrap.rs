pub fn bwrap_runs() -> bool {
    let runs = std::process::Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "true",
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !runs {
        eprintln!("skipped: bubblewrap cannot run here");
    }
    runs
}
