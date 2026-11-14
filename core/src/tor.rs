use anyhow::{Context, Result, ensure};
use rand::{RngCore, rngs::OsRng};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub struct Tor {
    child: Arc<Mutex<Child>>,
    control: Option<BufReader<TcpStream>>,
    pub proxy: SocketAddr,
    pub onion: String,
    pub onion_key: Zeroizing<String>,
}
impl Tor {
    pub fn child_handle(&self) -> Arc<Mutex<Child>> {
        self.child.clone()
    }
    pub fn start(binary: &Path, directory: &Path, backend: u16, key: Option<&str>) -> Result<Self> {
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let mut random = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(random.as_mut());
        let password = Zeroizing::new(hex::encode(*random));
        let output = Command::new(binary)
            .args(["--hash-password", &password])
            .arg("--DataDirectory")
            .arg(directory)
            .output()?;
        ensure!(output.status.success(), "Tor password setup failed");
        let hash = String::from_utf8(output.stdout)?
            .lines()
            .find(|l| l.starts_with("16:"))
            .context("Tor password setup failed")?
            .to_owned();
        let portfile = directory.join("control-port");
        let torrc = directory.join("torrc");
        fs::write(&torrc, b"# Managed session; no onion secrets on disk.\n")?;
        if portfile.exists() {
            fs::remove_file(&portfile)?;
        }
        let child = Command::new(binary)
            .arg("-f")
            .arg(&torrc)
            .args([
                "--RunAsDaemon",
                "0",
                "--ClientOnly",
                "1",
                "--AvoidDiskWrites",
                "1",
                "--SocksPort",
                "auto OnionTrafficOnly",
                "--HTTPTunnelPort",
                "0",
                "--DNSPort",
                "0",
                "--TransPort",
                "0",
                "--ControlPort",
                "auto",
                "--HashedControlPassword",
                &hash,
                "--CookieAuthentication",
                "0",
                "--DisableNetwork",
                "1",
                "--SafeLogging",
                "1",
                "--Log",
                "err stdout",
                "--__OwningControllerProcess",
                &std::process::id().to_string(),
            ])
            .arg("--DataDirectory")
            .arg(directory)
            .arg("--ControlPortWriteToFile")
            .arg(&portfile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut tor = Self {
            child: Arc::new(Mutex::new(child)),
            control: None,
            proxy: "127.0.0.1:1".parse()?,
            onion: String::new(),
            onion_key: Zeroizing::new(String::new()),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let address = loop {
            ensure!(
                tor.child.lock().unwrap().try_wait()?.is_none(),
                "Tor exited during startup"
            );
            if let Ok(s) = fs::read_to_string(&portfile)
                && let Some(p) = s.trim().strip_prefix("PORT=")
            {
                break p.parse::<SocketAddr>()?;
            }
            ensure!(Instant::now() < deadline, "Tor startup timed out");
            thread::sleep(Duration::from_millis(100));
        };
        ensure!(address.ip().is_loopback(), "unsafe control listener");
        let socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        socket.set_write_timeout(Some(Duration::from_secs(5)))?;
        tor.control = Some(BufReader::new(socket));
        let authentication =
            Zeroizing::new(format!("AUTHENTICATE {}", hex::encode(password.as_bytes())));
        tor.command(&authentication)?;
        tor.command("TAKEOWNERSHIP")?;
        let spec = Zeroizing::new(match key {
            Some(k) => {
                ensure!(
                    k.len() < 200
                        && k.starts_with("ED25519-V3:")
                        && k.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-:+/=".contains(&b)),
                    "invalid onion key"
                );
                k.to_owned()
            }
            None => "NEW:ED25519-V3".to_owned(),
        });
        let add = Zeroizing::new(format!(
            "ADD_ONION {} Port=443,127.0.0.1:{backend}",
            spec.as_str()
        ));
        let reply = tor.command(&add)?;
        tor.onion = format!(
            "{}.onion",
            reply
                .iter()
                .find_map(|s| s.strip_prefix("ServiceID="))
                .context("missing service")?
        );
        crate::contact::validate_onion(&tor.onion)?;
        tor.onion_key = Zeroizing::new(match key {
            Some(k) => k.to_owned(),
            None => reply
                .iter()
                .find_map(|s| s.strip_prefix("PrivateKey="))
                .context("missing onion key")?
                .to_owned(),
        });
        Ok(tor)
    }
    pub fn command(&mut self, command: &str) -> Result<Vec<Zeroizing<String>>> {
        ensure!(!command.contains(['\r', '\n']), "invalid control command");
        let reader = self.control.as_mut().context("Tor stopped")?;
        reader.get_mut().write_all(command.as_bytes())?;
        reader.get_mut().write_all(b"\r\n")?;
        let mut lines = Vec::new();
        for _ in 0..32 {
            let mut line = Zeroizing::new(String::new());
            let n = reader.by_ref().take(4097).read_line(&mut line)?;
            ensure!(
                n > 0 && n <= 4096 && line.starts_with("250"),
                "Tor control failed"
            );
            let end = line.as_bytes().get(3) == Some(&b' ');
            lines.push(Zeroizing::new(line[4..].trim_end().to_owned()));
            if end {
                return Ok(lines);
            }
        }
        anyhow::bail!("Tor control response too large")
    }
    pub fn bootstrapped(&mut self) -> Result<bool> {
        Ok(self.bootstrap_progress()? == 100)
    }
    pub fn enable_network(&mut self) -> Result<()> {
        self.command("SETCONF DisableNetwork=0")?;
        let lines = self.command("GETINFO net/listeners/socks")?;
        let proxy = lines
            .iter()
            .find_map(|s| s.strip_prefix("net/listeners/socks="))
            .context("missing SOCKS listener")?
            .trim_matches('"');
        self.proxy = proxy.parse()?;
        ensure!(self.proxy.ip().is_loopback(), "unsafe SOCKS listener");
        Ok(())
    }
    pub fn refresh_service(&mut self, host: &str) -> Result<()> {
        crate::contact::validate_onion(host)?;
        self.command(&format!("HSFETCH {}", &host[..56]))?;
        Ok(())
    }
    pub fn bootstrap_progress(&mut self) -> Result<u8> {
        let lines = self.command("GETINFO status/bootstrap-phase")?;
        Ok(lines
            .iter()
            .flat_map(|s| s.split_whitespace())
            .find_map(|s| s.strip_prefix("PROGRESS="))
            .context("missing bootstrap status")?
            .parse()?)
    }
    pub fn stop(&mut self) {
        self.control.take();
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl Drop for Tor {
    fn drop(&mut self) {
        self.stop();
    }
}
