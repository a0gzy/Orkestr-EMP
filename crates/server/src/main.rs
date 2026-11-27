use std::path::PathBuf;

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result=match args.as_slice(){
        [command,flag,path,listen_flag,listen,dns_flag,dns] if command=="init" && flag=="--data" && listen_flag=="--listen" && dns_flag=="--dns" => {
            listen.parse().map_err(|_|"INVALID_LISTEN_ADDRESS".into()).and_then(|listen|orkestr_server::runtime::init(&PathBuf::from(path),listen,dns.clone())).map(|()|println!("Initialized. Bootstrap token is CurrentUser DPAPI in bootstrap-admin.dpapi (7 days). No service/firewall/trust changes."))
        },
        [command,flag,path] if flag=="--data" && command=="run" => run(PathBuf::from(path)),
        [command,flag,path] if flag=="--data" && command=="service" => service(PathBuf::from(path)),
        [command,data_flag,dir,file_flag,file] if command=="stage-msi" && data_flag=="--data" && file_flag=="--file" => orkestr_server::deployment::stage_msi(&PathBuf::from(dir),&PathBuf::from(file)).map(|hash|println!("MSI_SHA256 {hash}")).map_err(str::to_string),
        [command,data_flag,dir,file_flag,file] if command=="stage-exe" && data_flag=="--data" && file_flag=="--file" => orkestr_server::deployment::stage_exe(&PathBuf::from(dir),&PathBuf::from(file)).map(|hash|println!("EXE_SHA256 {hash}")).map_err(str::to_string),
        [command,data_flag,dir,file_flag,file] if command=="stage-file" && data_flag=="--data" && file_flag=="--file" => orkestr_server::deployment::stage_file(&PathBuf::from(dir),&PathBuf::from(file)).map(|hash|println!("FILE_SHA256 {hash}")).map_err(str::to_string),
        _=>{println!("Orkestr EMP server\ninit --data <NEW-LOCAL-NTFS-DIRECTORY> --listen 127.0.0.1:7443 --dns localhost\nrun --data <EXISTING-DIRECTORY>\nservice --data <EXISTING-DIRECTORY> (SCM only)\nstage-msi --data <EXISTING-DIRECTORY> --file <ABSOLUTE-LOCAL-MSI>\nstage-exe --data <EXISTING-DIRECTORY> --file <ABSOLUTE-LOCAL-EXE>\nstage-file --data <EXISTING-DIRECTORY> --file <ABSOLUTE-LOCAL-FILE>\nNo automatic domain, firewall, WinRM or certificate trust changes.");Ok(())}
    };
    if let Err(code) = result {
        eprintln!("Server blocked: {code}");
        std::process::exit(1);
    }
}
fn run(path: PathBuf) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|_| "RUNTIME_FAILED")?;
    runtime.block_on(async {
        let handle = axum_server::Handle::new();
        let shutdown = handle.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(35)));
        });
        orkestr_server::runtime::serve_ready(&path, handle, |address| {
            println!("READY https://{address}")
        })
        .await
    })
}
#[cfg(not(windows))]
fn service(_: PathBuf) -> Result<(), String> {
    Err("WINDOWS_REQUIRED".into())
}
#[cfg(windows)]
fn service(path: PathBuf) -> Result<(), String> {
    SERVICE_PATH.set(path).map_err(|_| "SERVICE_STATE_FAILED")?;
    windows_service::service_dispatcher::start("OrkestrEMP", ffi_service_main)
        .map_err(|_| "SCM_START_FAILED".into())
}
#[cfg(windows)]
static SERVICE_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, service_main);
#[cfg(windows)]
fn service_main(_: Vec<std::ffi::OsString>) {
    use windows_service::{
        service::*,
        service_control_handler::{self, ServiceControlHandlerResult},
    };
    let handle = axum_server::Handle::new();
    let shutdown = handle.clone();
    let Ok(status) =
        service_control_handler::register("OrkestrEMP", move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(35)));
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
    else {
        return;
    };
    let report = |state, exit| {
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: exit,
            checkpoint: 0,
            wait_hint: std::time::Duration::from_secs(40),
            process_id: None,
        })
    };
    let _ = report(ServiceState::StartPending, ServiceExitCode::Win32(0));
    let result = (|| {
        let path = SERVICE_PATH.get().ok_or("SERVICE_PATH_MISSING")?;
        let runtime = tokio::runtime::Runtime::new().map_err(|_| "RUNTIME_FAILED")?;
        runtime.block_on(orkestr_server::runtime::serve_ready(path, handle, |_| {
            let _ = report(ServiceState::Running, ServiceExitCode::Win32(0));
        }))
    })();
    let _ = report(
        ServiceState::Stopped,
        ServiceExitCode::Win32(if result.is_ok() { 0 } else { 1 }),
    );
}
