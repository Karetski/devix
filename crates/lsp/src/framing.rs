//! LSP JSON-RPC framing over an async byte stream.
//!
//! Each message is `Content-Length: N\r\n\r\n` followed by N bytes of JSON.
//! Optional `Content-Type` headers are accepted and ignored. The reader
//! returns owned `Vec<u8>` payloads — callers parse to `serde_json::Value`
//! (or stronger types) themselves.

use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

pub struct FrameReader<R> {
    inner: BufReader<R>,
    line: String,
}

impl<R: tokio::io::AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner: BufReader::new(inner),
            line: String::with_capacity(64),
        }
    }

    /// Read one framed message. Returns `Ok(None)` on a clean EOF before any
    /// header bytes; `Err` on a partial frame or malformed header.
    pub async fn read_frame(&mut self) -> Result<Option<Vec<u8>>> {
        let mut content_length: Option<usize> = None;
        let mut saw_any_header = false;

        loop {
            self.line.clear();
            let n = self.inner.read_line(&mut self.line).await?;
            if n == 0 {
                if !saw_any_header {
                    return Ok(None);
                }
                bail!("EOF inside header block");
            }
            saw_any_header = true;

            // Header lines end in \r\n. A bare \r\n terminates the block.
            let trimmed = self.line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            let (name, value) = trimmed
                .split_once(':')
                .ok_or_else(|| anyhow!("malformed header line: {:?}", trimmed))?;
            if name.eq_ignore_ascii_case("Content-Length") {
                let v: usize = value
                    .trim()
                    .parse()
                    .with_context(|| format!("Content-Length not a usize: {:?}", value))?;
                content_length = Some(v);
            }
            // Other headers (Content-Type) are ignored.
        }

        let len = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
        let mut buf = vec![0u8; len];
        self.inner
            .read_exact(&mut buf)
            .await
            .with_context(|| format!("reading {len}-byte body"))?;
        Ok(Some(buf))
    }
}

/// Write a single framed message. Caller passes the JSON body as bytes.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(header.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn roundtrip_single_frame() {
        let (mut wr, rd) = duplex(4096);
        let mut reader = FrameReader::new(rd);
        let body = br#"{"jsonrpc":"2.0","method":"x"}"#;
        write_frame(&mut wr, body).await.unwrap();
        drop(wr);
        let got = reader.read_frame().await.unwrap().unwrap();
        assert_eq!(got, body);
    }

    #[tokio::test]
    async fn roundtrip_multiple_frames() {
        let (mut wr, rd) = duplex(4096);
        let mut reader = FrameReader::new(rd);
        for i in 0..5 {
            let body = format!(r#"{{"i":{i}}}"#);
            write_frame(&mut wr, body.as_bytes()).await.unwrap();
        }
        drop(wr);
        for i in 0..5 {
            let got = reader.read_frame().await.unwrap().unwrap();
            assert_eq!(got, format!(r#"{{"i":{i}}}"#).as_bytes());
        }
        assert!(reader.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ignores_content_type_header() {
        let (mut wr, rd) = duplex(4096);
        let mut reader = FrameReader::new(rd);
        let body = br#"{"x":1}"#;
        let header = format!(
            "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n",
            body.len()
        );
        wr.write_all(header.as_bytes()).await.unwrap();
        wr.write_all(body).await.unwrap();
        wr.flush().await.unwrap();
        drop(wr);
        let got = reader.read_frame().await.unwrap().unwrap();
        assert_eq!(got, body);
    }

    #[tokio::test]
    async fn case_insensitive_content_length() {
        let (mut wr, rd) = duplex(4096);
        let mut reader = FrameReader::new(rd);
        let body = br#"{}"#;
        wr.write_all(b"content-length: 2\r\n\r\n").await.unwrap();
        wr.write_all(body).await.unwrap();
        wr.flush().await.unwrap();
        drop(wr);
        let got = reader.read_frame().await.unwrap().unwrap();
        assert_eq!(got, body);
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (wr, rd) = duplex(64);
        let mut reader = FrameReader::new(rd);
        drop(wr);
        assert!(reader.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn missing_content_length_errors() {
        let (mut wr, rd) = duplex(64);
        let mut reader = FrameReader::new(rd);
        wr.write_all(b"Content-Type: x\r\n\r\n").await.unwrap();
        wr.flush().await.unwrap();
        drop(wr);
        let err = reader.read_frame().await.expect_err("must error");
        assert!(err.to_string().contains("Content-Length"));
    }
}
