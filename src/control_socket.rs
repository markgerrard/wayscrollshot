use anyhow::{Context, Result};
use std::{os::unix::net::UnixDatagram, path::PathBuf};

pub struct CaptureSocket {
    socket: UnixDatagram,
    path: PathBuf,
}
impl CaptureSocket {
    pub fn open(toggle: bool) -> Result<Option<Self>> {
        let path =
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR missing")?)
                .join("wayscrollshot-control.sock");
        let client = UnixDatagram::unbound()?;
        if client.connect(&path).is_ok() {
            if toggle {
                client.send(b"save")?;
                return Ok(None);
            }
            anyhow::bail!("Capture already running; use --toggle to finish it");
        }
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let socket = UnixDatagram::bind(&path)?;
        socket.set_nonblocking(true)?;
        Ok(Some(Self { socket, path }))
    }
    pub fn finish_requested(&self) -> bool {
        let mut buf = [0; 16];
        matches!(self.socket.recv(&mut buf), Ok(4)) && &buf[..4] == b"save"
    }
}
impl Drop for CaptureSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
