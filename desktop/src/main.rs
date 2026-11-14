use anyhow::{Context, Result, ensure};
use psychotalk_desktop::{
    engine::Engine,
    envelopes::{Envelopes, OsStore, private_dir},
    server::{App, router},
};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
fn base() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("PSYCHOTALK_DATA_DIR") {
        let p = PathBuf::from(p);
        ensure!(p.is_absolute(), "PSYCHOTALK_DATA_DIR must be absolute");
        return Ok(p);
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not configured")?);
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Psychotalk")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join("psychotalk")
    })
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Psychotalk: {e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--version") {
        println!("Psychotalk {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let executable = std::fs::canonicalize(std::env::current_exe()?)?;
    let mut root = base()?;
    let mut tor = executable
        .parent()
        .context("missing installation directory")?
        .join("libexec/tor");
    let mut no_open = false;
    let mut operation = "open";
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                i += 1;
                root = PathBuf::from(args.get(i).context("--data-dir requires a path")?);
            }
            "--tor" => {
                i += 1;
                tor = PathBuf::from(args.get(i).context("--tor requires a path")?);
            }
            "--no-open" => no_open = true,
            "stop" => operation = "stop",
            "start" => {}
            "update" | "uninstall" => {
                let installer = executable.parent().unwrap().join("install.sh");
                let status = std::process::Command::new("sh")
                    .arg(installer)
                    .arg(if args[i] == "update" {
                        "--update"
                    } else {
                        "--uninstall"
                    })
                    .status()?;
                ensure!(status.success(), "installer failed");
                return Ok(());
            }
            "--help" => {
                println!(
                    "psychotalk [start|stop|update|uninstall] [--data-dir PATH] [--tor PATH] [--no-open]"
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument; use --help"),
        };
        i += 1;
    }
    ensure!(root.is_absolute(), "data directory must be absolute");
    private_dir(&root)?;
    let socket_path = root.join("control.sock");
    let instance = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance.lock"))?;
    if instance.try_lock().is_err() {
        let mut socket = UnixStream::connect(&socket_path)
            .context("O programa já está aberto; aguarde ou use psychotalk stop.")?;
        socket.set_read_timeout(Some(Duration::from_secs(15)))?;
        socket.write_all(operation.as_bytes())?;
        socket.shutdown(std::net::Shutdown::Write)?;
        let mut answer = String::new();
        socket.take(100).read_to_string(&mut answer)?;
        ensure!(answer == "ok", "Não foi possível abrir a janela existente.");
        if operation == "stop" {
            let end = std::time::Instant::now() + Duration::from_secs(15);
            while instance.try_lock().is_err() {
                ensure!(
                    std::time::Instant::now() < end,
                    "A sessão ainda está encerrando; tente novamente."
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        return Ok(());
    }
    if operation == "stop" {
        return Ok(());
    }
    ensure!(
        tor.is_file(),
        "Tor empacotado não encontrado. Reinstale o pacote desktop ou use --tor PATH."
    );
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let control = UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    control.set_nonblocking(true)?;
    let engine = Arc::new(Engine::new(
        Envelopes::new(&root, OsStore::new(&root)?)?,
        tor,
    )?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let app = App::new(engine.clone(), listener.local_addr()?.to_string());
    let url = app.url();
    if no_open {
        println!("{url}");
    } else {
        webbrowser::open(&url)
            .context("Não foi possível abrir o navegador. Use --no-open para abrir manualmente.")?;
        println!("Psychotalk aberto no navegador. Ctrl+C encerra o programa.");
    }
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let notify = shutdown.clone();
    let app_control = app.clone();
    let controller = std::thread::spawn(move || {
        loop {
            match control.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut bytes = Vec::new();
                    if Read::by_ref(&mut stream)
                        .take(16)
                        .read_to_end(&mut bytes)
                        .is_ok()
                    {
                        if bytes == b"stop" {
                            app_control.reset();
                            let _ = stream.write_all(b"ok");
                            notify.notify_one();
                            break;
                        }
                        if bytes == b"open" {
                            app_control.reset();
                            let ok = webbrowser::open(&app_control.url()).is_ok();
                            let _ = stream.write_all(if ok { b"ok" } else { b"error" });
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(_) => break,
            }
            if Arc::strong_count(&notify) == 1 {
                break;
            }
        }
    });
    let close_engine = engine.clone();
    let close_notify = shutdown.clone();
    let signals = tokio::spawn(async move {
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = terminate.recv()=>{}}
        close_engine.revoke();
        close_notify.notify_one();
    });
    let outcome = axum::serve(listener, router(app))
        .with_graceful_shutdown({
            let shutdown = shutdown.clone();
            async move { shutdown.notified().await }
        })
        .await;
    engine.lock();
    signals.abort();
    drop(shutdown);
    let _ = controller.join();
    let _ = std::fs::remove_file(socket_path);
    drop(instance);
    outcome?;
    Ok(())
}
