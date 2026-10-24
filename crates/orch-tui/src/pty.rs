use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;

use orch_git::ENV_REDIRECTING_GIT;
use orch_protocol::Size;
use portable_pty::{
    ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::event::{Event, PaneId, ShellEvent};

struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Default)]
pub(crate) struct ShellPtys(HashMap<PaneId, Pty>);

impl ShellPtys {
    pub fn open(
        &mut self,
        pane: PaneId,
        command: Option<&str>,
        cwd: &Path,
        size: Size,
        events: &UnboundedSender<Event>,
    ) -> io::Result<()> {
        let pty = native_pty_system()
            .openpty(pty_size(size))
            .map_err(io::Error::other)?;
        let mut builder = match command {
            Some(command) => {
                let mut builder = CommandBuilder::new("sh");
                builder.args(["-c", command]);
                builder
            }
            None => CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "sh".into())),
        };
        builder.cwd(cwd);
        builder.env("TERM", "xterm-256color");
        for key in ENV_REDIRECTING_GIT {
            builder.env_remove(key);
        }
        let mut child = pty.slave.spawn_command(builder).map_err(io::Error::other)?;
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pty.master.take_writer().map_err(io::Error::other)?;
        let killer = child.clone_killer();
        let events = events.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                let event = ShellEvent::Output(buffer[..read].to_vec());
                if events.send(Event::Shell { pane, event }).is_err() {
                    break;
                }
            }
            let status = match child.wait() {
                Ok(status) => describe(&status),
                Err(err) => err.to_string(),
            };
            let event = ShellEvent::Exited(status);
            let _ = events.send(Event::Shell { pane, event });
        });
        let pty = Pty {
            master: pty.master,
            writer,
            killer,
        };
        self.0.insert(pane, pty);
        Ok(())
    }

    pub fn input(&mut self, pane: PaneId, bytes: &[u8]) {
        if let Some(pty) = self.0.get_mut(&pane) {
            let _ = pty
                .writer
                .write_all(bytes)
                .and_then(|()| pty.writer.flush());
        }
    }

    pub fn resize(&mut self, pane: PaneId, size: Size) {
        if let Some(pty) = self.0.get(&pane) {
            let _ = pty.master.resize(pty_size(size));
        }
    }

    pub fn close(&mut self, pane: PaneId) {
        if let Some(mut pty) = self.0.remove(&pane) {
            let _ = pty.killer.kill();
        }
    }
}

impl Drop for ShellPtys {
    fn drop(&mut self) {
        for pty in self.0.values_mut() {
            let _ = pty.killer.kill();
        }
    }
}

fn describe(status: &ExitStatus) -> String {
    match status.signal() {
        Some(signal) => format!("killed by {signal}"),
        None => format!("exit status: {}", status.exit_code()),
    }
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;
    use crate::event::ShellEvent;

    const SIZE: Size = Size { rows: 10, cols: 40 };

    fn collect(inbox: &mut UnboundedReceiver<Event>) -> (String, Option<String>) {
        let mut output = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match inbox.try_recv() {
                Ok(Event::Shell {
                    event: ShellEvent::Output(bytes),
                    ..
                }) => output.extend(bytes),
                Ok(Event::Shell {
                    event: ShellEvent::Exited(status),
                    ..
                }) => return (String::from_utf8_lossy(&output).into_owned(), Some(status)),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        (String::from_utf8_lossy(&output).into_owned(), None)
    }

    #[test]
    fn a_command_runs_in_its_cwd_and_reports_its_exit() {
        let dir = tempfile::tempdir().unwrap();
        let (events, mut inbox) = unbounded_channel();
        let mut ptys = ShellPtys::default();
        let command = "pwd; exit 3";
        ptys.open(PaneId(1), Some(command), dir.path(), SIZE, &events)
            .unwrap();
        let (output, status) = collect(&mut inbox);
        let cwd = dir.path().canonicalize().unwrap();
        assert!(output.contains(cwd.to_str().unwrap()), "{output:?}");
        assert_eq!(status.as_deref(), Some("exit status: 3"));
    }

    #[test]
    fn input_reaches_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let (events, mut inbox) = unbounded_channel();
        let mut ptys = ShellPtys::default();
        ptys.open(
            PaneId(1),
            Some("read line; echo got-$line"),
            dir.path(),
            SIZE,
            &events,
        )
        .unwrap();
        ptys.input(PaneId(1), b"hello\r");
        let (output, status) = collect(&mut inbox);
        assert!(output.contains("got-hello"), "{output:?}");
        assert_eq!(status.as_deref(), Some("exit status: 0"));
    }

    #[test]
    fn close_kills_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let (events, mut inbox) = unbounded_channel();
        let mut ptys = ShellPtys::default();
        ptys.open(PaneId(1), Some("sleep 30"), dir.path(), SIZE, &events)
            .unwrap();
        ptys.close(PaneId(1));
        let (_, status) = collect(&mut inbox);
        assert!(status.is_some());
    }

    #[test]
    fn resize_changes_the_terminal_size() {
        let dir = tempfile::tempdir().unwrap();
        let (events, mut inbox) = unbounded_channel();
        let mut ptys = ShellPtys::default();
        ptys.open(
            PaneId(1),
            Some("read _; stty size"),
            dir.path(),
            SIZE,
            &events,
        )
        .unwrap();
        ptys.resize(PaneId(1), Size { rows: 12, cols: 50 });
        ptys.input(PaneId(1), b"\r");
        let (output, _) = collect(&mut inbox);
        assert!(output.contains("12 50"), "{output:?}");
    }
}
