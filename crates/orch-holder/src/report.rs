use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use orch_agent::GuardAnswer;

use crate::frame::{read_frame, write_frame};
use crate::{FromHolder, ToHolder};

const SEND_TIMEOUT: Duration = Duration::from_millis(500);

pub fn report(socket: &Path, message: &ToHolder) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(SEND_TIMEOUT))?;
    write_frame(&mut stream, message)
}

pub fn request_guard(socket: &Path, payload: &str) -> GuardAnswer {
    let exchange = || -> io::Result<GuardAnswer> {
        let mut stream = UnixStream::connect(socket)?;
        stream.set_write_timeout(Some(SEND_TIMEOUT))?;
        write_frame(
            &mut stream,
            &ToHolder::Guard {
                payload: payload.into(),
            },
        )?;
        match read_frame(&mut stream)? {
            Some(FromHolder::GuardAnswer { answer }) => Ok(answer),
            _ => Ok(GuardAnswer::Ask),
        }
    };
    exchange().unwrap_or(GuardAnswer::Ask)
}
