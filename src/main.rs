use ctrlc;
use std::{thread, time::Duration};

use win_dbg_logger::output_debug_string;

async fn hi() -> impl actix_web::Responder {
    "Hello!\r\n"
}

struct AppFinisher {}

impl Drop for AppFinisher {
    fn drop(&mut self) {
        output_debug_string("AppFinisher stop wait. 3 seconds");
        thread::sleep(Duration::from_secs(3));
        output_debug_string("AppFinisher finish!!");
    }
}

//#[actix_web::main]
pub async fn service_main(rx: std::sync::mpsc::Receiver<()>) -> std::io::Result<()> {
    let _finisher = AppFinisher {};
    let server = actix_web::HttpServer::new(move || {
        actix_web::App::new().route("/", actix_web::web::get().to(hi))
    })
    .bind("0.0.0.0:3000")?
    .run();

    rx.recv().unwrap();
    server.stop(true).await;

    Ok(())
}

pub async fn wnd_service_main(rx: std::sync::mpsc::Receiver<()>) -> std::io::Result<()> {
    service_main(rx).await?;
    unsafe {
        services::SHUTDON_FLG = true;
    }
    Ok(())
}

/// 各プラットフォーム共通のメイン関数 (除: Windows Service)
fn platform_common_main() -> std::io::Result<()> {
    let mut sys = actix_web::rt::System::new(services::SERVICE_NAME);

    let (tx, rx) = std::sync::mpsc::channel();
    // 敢えて unwrap している。
    let _ = ctrlc::set_handler(move || {
        output_debug_string("recieve ServiceState::Stopped message -- 1");

        tx.send(()).unwrap();
        loop {
            unsafe {
                if services::SHUTDON_FLG == true {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }

        output_debug_string("recieve ServiceState::Stopped message -- 2");
    })
    .unwrap();

    let _ = sys.block_on(wnd_service_main(rx));
    Ok(())
}

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER).unwrap();
    //log::set_max_level(log::LevelFilter::);

    win_dbg_logger::output_debug_string("main start.");
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "install" {
        println!("install service");
        services::install_service()
    } else if args.len() > 1 && args[1] == "uninstall" {
        println!("uninstall service");
        services::uninstall_service()
    } else if args.len() > 1 && args[1] == "test" {
        platform_common_main().map_err(|e| windows_service::Error::Winapi(e))
    } else {
        services::run()
    }
}

#[cfg(not(windows))]
fn main() -> std::io::Result<()> {
    platform_common_main()
}

#[cfg(windows)]
mod services {
    use std::ffi::OsString;
    use std::{sync::mpsc, thread, time::Duration};
    use win_dbg_logger::output_debug_string;
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher, Result};

    use windows_service::{
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
    };
    pub const SERVICE_NAME: &'static str = "actix_wnd_service_sample";
    const SERVICE_DISP_NAME: &str = "Actix Windows Service Sample";
    const SERVICE_DESCRIPTION: &str = "test for Actix-web on the windows service example.";

    const SERVICE_PROCESS_NAME: &str = "rs_wnd_service2.exe";

    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    pub static mut SHUTDON_FLG: bool = false;

    pub fn run() -> Result<()> {
        // Register generated `ffi_service_main` with the system and start the service, blocking
        // this thread until the service is stopped.
        output_debug_string("run()");
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    // Generate the windows service boilerplate.
    // The boilerplate contains the low-level service entry function (ffi_service_main) that parses
    // incoming service arguments into Vec<OsString> and passes them to user defined service
    // entry (my_service_main).
    define_windows_service!(ffi_service_main, my_service_main);

    // Service entry function which is called on background thread by the system with service
    // parameters. There is no stdout or stderr at this point so make sure to configure the log
    // output to file if needed.
    pub fn my_service_main(_arguments: Vec<OsString>) {
        output_debug_string("my_service_main");
        if let Err(_e) = run_service() {
            // Handle the error, by logging or something.
        }
    }

    pub fn install_service() -> Result<()> {
        let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

        // This example installs the service defined in `examples/ping_service.rs`.
        // In the real world code you would set the executable path to point to your own binary
        // that implements windows service.
        let service_binary_path = ::std::env::current_exe()
            .unwrap()
            .with_file_name(SERVICE_PROCESS_NAME);

        let service_info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(SERVICE_DISP_NAME),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::OnDemand,
            error_control: ServiceErrorControl::Normal,
            executable_path: service_binary_path,
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None, // run as System
            account_password: None,
        };
        let service =
            service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
        service.set_description(SERVICE_DESCRIPTION)?;
        Ok(())
    }

    pub fn uninstall_service() -> Result<()> {
        let manager_access = ServiceManagerAccess::CONNECT;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

        let service_access =
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
        let service = service_manager.open_service(SERVICE_NAME, service_access)?;

        let service_status = service.query_status()?;
        if service_status.current_state != ServiceState::Stopped {
            service.stop()?;
            // Wait for service to stop
            thread::sleep(Duration::from_secs(1));
        }

        service.delete()?;
        Ok(())
    }

    pub fn run_service() -> anyhow::Result<()> {
        let mut sys = actix_web::rt::System::new(SERVICE_NAME);

        let (mut send_stop, recv_stop) = {
            let (p, c) = mpsc::channel();
            (Some(p), c)
        };

        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            output_debug_string(&format!("control_event: {:?}", control_event));
            match control_event {
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                ServiceControl::Stop => {
                    send_stop.take().unwrap().send(()).unwrap();
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
            .map_err(|e| anyhow::anyhow!("service_control_handler::register failed: {:?}", e))?;

        output_debug_string("service_control_handler::register done!!");

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            for _ in recv_stop {
                output_debug_string("recieve ServiceState::Stopped message -- 1");

                tx.send(()).unwrap();

                status_handle
                    .set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::StopPending,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::Win32(0),
                        checkpoint: 0,
                        wait_hint: Duration::from_secs(20),
                        process_id: None,
                    })
                    .unwrap();

                loop {
                    unsafe {
                        if SHUTDON_FLG == true {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }

                status_handle
                    .set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::Stopped,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::Win32(0),
                        checkpoint: 0,
                        wait_hint: Duration::default(),
                        process_id: None,
                    })
                    .unwrap();

                output_debug_string("recieve ServiceState::Stopped message -- 2");
                actix_web::rt::System::current().stop();
            }
        });

        output_debug_string("service_main start.");

        let _ = sys.block_on(super::wnd_service_main(rx));

        output_debug_string("service_main stopped.");

        Ok(())
    }
}
