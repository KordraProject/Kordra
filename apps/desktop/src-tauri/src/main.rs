#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Desktop shell around the shared Kordra Web UI and packaged headless daemon.
//! The application owns the child process it starts. A preexisting localhost
//! daemon is reused, never terminated on exit.
use std::{path::PathBuf, process::{Child, Command, Stdio}, sync::Mutex, time::Duration};
use tauri::{Manager, RunEvent};

struct Sidecar(Mutex<Option<Child>>);

fn daemon_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let current = std::env::current_exe()?;
    let name = if cfg!(windows) { "kordra-daemon.exe" } else { "kordra-daemon" };
    Ok(current.with_file_name(name))
}
fn daemon_online() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = "127.0.0.1:8079".parse().expect("static loopback address");
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}
fn launch_daemon(app: &tauri::App) -> Result<Option<Child>, Box<dyn std::error::Error>> {
    if daemon_online() { return Ok(None); }
    let path = daemon_path()?;
    if !path.is_file() { return Err(format!("Kordra daemon sidecar missing: {}", path.display()).into()); }
    let data = app.path().app_data_dir()?;
    let output = app.path().download_dir().unwrap_or_else(|_| data.join("downloads"));
    std::fs::create_dir_all(&data)?;
    std::fs::create_dir_all(&output)?;
    let child = Command::new(path)
        .env("KORDRA_LISTEN", "127.0.0.1:8079")
        .env("KORDRA_WEB_ENABLED", "false")
        .env("KORDRA_DATA_DIR", data)
        .env("KORDRA_DOWNLOAD_DIR", output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Some(child))
}
fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let child = launch_daemon(app)?;
            app.manage(Sidecar(Mutex::new(child)));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("could not initialize Kordra desktop");
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            if let Some(sidecar) = app_handle.try_state::<Sidecar>() {
                if let Ok(mut guard) = sidecar.0.lock() {
                    if let Some(mut child) = guard.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
        }
    });
}
