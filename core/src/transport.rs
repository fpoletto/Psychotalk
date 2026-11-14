use crate::contact::validate_onion;
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

pub const MAX_FRAME: usize = 8192;

/// The application has exactly one outbound socket path: a loopback Tor SOCKS proxy.
pub fn onion_stream(proxy: SocketAddr, onion: &str) -> Result<TcpStream> {
    validate_onion(onion)?;
    ensure!(proxy.ip().is_loopback(), "loopback proxy required");
    let mut s = TcpStream::connect_timeout(&proxy, Duration::from_secs(5))?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    s.write_all(&[5, 1, 0])?;
    let mut reply = [0; 2];
    s.read_exact(&mut reply)?;
    ensure!(reply == [5, 0], "SOCKS unavailable");
    let mut request = vec![5, 1, 0, 3, onion.len() as u8];
    request.extend_from_slice(onion.as_bytes());
    request.extend_from_slice(&443u16.to_be_bytes());
    s.write_all(&request)?;
    let mut head = [0; 4];
    s.read_exact(&mut head)?;
    ensure!(
        head[0] == 5 && head[1] == 0 && head[2] == 0,
        "onion unavailable"
    );
    let length = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut b = [0];
            s.read_exact(&mut b)?;
            b[0] as usize
        }
        _ => anyhow::bail!("invalid SOCKS reply"),
    };
    let mut ignored = vec![0; length + 2];
    s.read_exact(&mut ignored)?;
    Ok(s)
}
pub fn write_frame(s: &mut impl Write, bytes: &[u8]) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_FRAME,
        "invalid frame size"
    );
    s.write_all(&(bytes.len() as u32).to_be_bytes())?;
    s.write_all(bytes)?;
    s.flush()?;
    Ok(())
}
pub fn read_frame(s: &mut impl Read) -> Result<Vec<u8>> {
    let mut head = [0; 4];
    s.read_exact(&mut head)?;
    let len = u32::from_be_bytes(head) as usize;
    ensure!(len > 0 && len <= MAX_FRAME, "invalid frame size");
    let mut bytes = vec![0; len];
    s.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_before_allocation() {
        assert!(read_frame(&mut &u32::MAX.to_be_bytes()[..]).is_err());
    }
    #[test]
    fn no_remote_proxy() {
        assert!(onion_stream("1.1.1.1:9050".parse().unwrap(), "localhost").is_err());
    }
}
