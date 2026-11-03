//! Windows service mode (`NobetPanel.exe --service`).
//!
//! The service starts with Windows, before anyone signs in, and keeps the
//! panel up to date after power cuts and reboots. It runs the same worker as
//! the window in automatic mode only and shares the data folder
//! `C:\ProgramData\NobetPanel` with it. While the service runs, the window
//! leaves automatic sending to it and only performs actions the user starts;
//! a machine-wide lock keeps the two from talking to the panel at once.

use std::ffi::OsString;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::store::{self, Store};
use crate::worker::{AutoMode, Event, Worker};

pub const SERVICE_NAME: &str = "NobetPanel";

define_windows_service!(ffi_service_main, service_main);

pub fn run() {
    store::set_service_mode();
    if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        // Started from a console instead of the service manager.
        crate::logfile::write("ERROR", &format!("Servis başlatılamadı (yalnızca Windows servis yöneticisi başlatabilir): {e}"));
    }
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        crate::logfile::write("ERROR", &format!("Servis hatası: {e}"));
    }
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn run_service() -> windows_service::Result<()> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let handler = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    handler.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN))?;

    let _mark = crate::platform::mark_service_running();
    let (store, warn) = Store::load();
    crate::logfile::write("INFO", &format!("Servis {} başladı. Veri: {}", env!("CARGO_PKG_VERSION"), store::data_path().display()));
    if let Some(w) = warn {
        crate::logfile::write("WARN", &w);
    }
    // The worker already writes its log lines to the log file.
    let emit: crate::worker::Emit = Arc::new(|e: Event| {
        if let Event::Done { ok: false, title, detail } = e {
            crate::logfile::write("ERROR", &format!("{title}: {detail}"));
        }
    });
    let worker = Worker::spawn(Arc::new(Mutex::new(store)), emit, AutoMode::Service);
    let _ = stop_rx.recv();
    handler.set_service_status(status(ServiceState::StopPending, ServiceControlAccept::empty()))?;
    worker.shutdown();
    crate::logfile::write("INFO", "Servis durduruldu.");
    handler.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    Ok(())
}
