use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_FRAME: usize = 64 << 20;

fn encode<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    let body = serde_json::to_vec(message)?;
    let len = u32::try_from(body.len()).map_err(io::Error::other)?;
    let mut frame = len.to_be_bytes().to_vec();
    frame.extend(body);
    Ok(frame)
}

fn frame_len(header: [u8; 4]) -> io::Result<usize> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    Ok(len)
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> io::Result<T> {
    serde_json::from_slice(body).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    writer.write_all(&encode(message)?)?;
    writer.flush()
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<Option<T>> {
    let mut header = [0; 4];
    match reader.read_exact(&mut header) {
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        other => other?,
    }
    let mut body = vec![0; frame_len(header)?];
    reader.read_exact(&mut body)?;
    decode(&body).map(Some)
}

pub async fn write_frame_async<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &T,
) -> io::Result<()> {
    writer.write_all(&encode(message)?).await?;
    writer.flush().await
}

pub async fn read_frame_async<T: DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> io::Result<Option<T>> {
    let mut header = [0; 4];
    match reader.read_exact(&mut header).await {
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        other => other?,
    };
    let mut body = vec![0; frame_len(header)?];
    reader.read_exact(&mut body).await?;
    decode(&body).map(Some)
}
