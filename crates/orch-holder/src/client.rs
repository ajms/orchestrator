use std::collections::VecDeque;
use std::io;
use std::path::Path;

use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::frame::{read_frame_async, write_frame_async};
use crate::{FromHolder, Hello, PROTOCOL_VERSION, ScreenSnapshot, ToHolder};

pub struct HolderClient {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    pending: VecDeque<FromHolder>,
}

impl HolderClient {
    pub async fn connect(socket: &Path) -> io::Result<Self> {
        let (reader, writer) = UnixStream::connect(socket).await?.into_split();
        Ok(Self {
            reader,
            writer,
            pending: VecDeque::new(),
        })
    }

    pub async fn attach(&mut self) -> io::Result<Hello> {
        self.send(&ToHolder::Attach {
            version: PROTOCOL_VERSION,
        })
        .await?;
        match self.read().await? {
            Some(FromHolder::Hello(hello)) => Ok(hello),
            other => Err(unexpected(other)),
        }
    }

    pub async fn send(&mut self, message: &ToHolder) -> io::Result<()> {
        write_frame_async(&mut self.writer, message).await
    }

    pub async fn recv(&mut self) -> io::Result<Option<FromHolder>> {
        match self.pending.pop_front() {
            Some(message) => Ok(Some(message)),
            None => self.read().await,
        }
    }

    pub async fn snapshot(&mut self) -> io::Result<ScreenSnapshot> {
        self.send(&ToHolder::Snapshot).await?;
        loop {
            match self.read().await? {
                Some(FromHolder::Screen(snapshot)) => return Ok(snapshot),
                Some(other) => self.pending.push_back(other),
                None => return Err(unexpected(None)),
            }
        }
    }

    pub fn into_split(self) -> (HolderReader, OwnedWriteHalf) {
        let reader = HolderReader {
            reader: self.reader,
            pending: self.pending,
        };
        (reader, self.writer)
    }

    async fn read(&mut self) -> io::Result<Option<FromHolder>> {
        read_frame_async(&mut self.reader).await
    }
}

pub struct HolderReader {
    reader: OwnedReadHalf,
    pending: VecDeque<FromHolder>,
}

impl HolderReader {
    pub async fn recv(&mut self) -> io::Result<Option<FromHolder>> {
        match self.pending.pop_front() {
            Some(message) => Ok(Some(message)),
            None => read_frame_async(&mut self.reader).await,
        }
    }
}

fn unexpected(message: Option<FromHolder>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected holder message: {message:?}"),
    )
}
