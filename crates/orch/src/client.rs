use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use orch_holder::default_runtime_dir;
use orch_protocol::{Client, ConnectError, Reply, Request, RequestError, connect_or_spawn_daemon};

#[derive(Debug)]
pub enum ClientError {
    NoBinary(io::Error),
    Runtime(io::Error),
    Connect(ConnectError),
    Lost(io::Error),
    Refused(RequestError),
    Unexpected(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBinary(err) => write!(f, "cannot locate the orch binary: {err}"),
            Self::Runtime(err) => write!(f, "cannot start the async runtime: {err}"),
            Self::Connect(err) => err.fmt(f),
            Self::Lost(err) => write!(f, "lost the Daemon: {err}"),
            Self::Refused(err) => err.fmt(f),
            Self::Unexpected(reply) => write!(f, "unexpected reply from the Daemon: {reply}"),
        }
    }
}

impl std::error::Error for ClientError {}

pub fn orch_program() -> Result<PathBuf, ClientError> {
    std::env::current_exe().map_err(ClientError::NoBinary)
}

pub fn block_on<T>(work: impl Future<Output = T>) -> Result<T, ClientError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(ClientError::Runtime)?;
    Ok(runtime.block_on(work))
}

pub fn request(request: Request) -> Result<Reply, ClientError> {
    let program = orch_program()?;
    block_on(async {
        let mut client: Client = connect_or_spawn_daemon(&default_runtime_dir(), &program)
            .await
            .map_err(ClientError::Connect)?;
        client
            .request(request)
            .await
            .map_err(ClientError::Lost)?
            .map_err(ClientError::Refused)
    })?
}

pub fn expect<T>(
    request: Request,
    pick: impl FnOnce(Reply) -> Option<T>,
) -> Result<T, ClientError> {
    let reply = self::request(request)?;
    let shown = format!("{reply:?}");
    pick(reply).ok_or(ClientError::Unexpected(shown))
}

pub fn fail(command: &str, code: u8, message: impl fmt::Display) -> ExitCode {
    eprintln!("orch {command}: {message}");
    ExitCode::from(code)
}
