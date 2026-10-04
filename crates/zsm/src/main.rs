#![windows_subsystem = "windows"]

mod app;
mod cmdline;
mod console;
mod dialog;
mod format;
mod summary;

use native_windows_gui as nwg;
use zsm_core::{paths, win};

use app::{App, AppOptions, APP_NAME};

const INSTANCE_MUTEX: &str = r"Local\ZServiceManager.SingleInstance";
const ACTIVATE_MESSAGE: &str = "ZServiceManager.Activate";

fn init_logging() {
    let path = paths::app_log_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let config = simplelog::ConfigBuilder::new()
            .set_time_offset_to_local()
            .unwrap_or_else(|b| b)
            .build();
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, config, file);
    }
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        log::error!(
            "panic in thread {}: {info}\n{}",
            thread.name().unwrap_or("?"),
            std::backtrace::Backtrace::force_capture()
        );
    }));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pid) = args
        .iter()
        .position(|a| a == "--ctrl-break")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
    {
        std::process::exit(i32::from(win::send_ctrl_break_attached(pid).is_err()));
    }
    let tray_mode = args.iter().any(|a| a == "--tray");

    init_logging();
    log::info!("{APP_NAME} v{} 시작 (args: {args:?})", zsm_core::VERSION);

    let activate_message = win::register_message(ACTIVATE_MESSAGE);
    let _instance = match win::acquire_single_instance(INSTANCE_MUTEX) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            log::info!("이미 실행 중인 인스턴스를 활성화하고 종료");
            win::broadcast_message(activate_message);
            return;
        }
        Err(e) => {
            log::error!("단일 인스턴스 mutex 생성 실패: {e}");
            None
        }
    };
    win::request_early_shutdown_notification();

    if let Err(e) = nwg::init() {
        log::error!("GUI 초기화 실패: {e}");
        return;
    }
    let _ = nwg::Font::set_global_family("Segoe UI");

    let app = match App::build(AppOptions {
        tray_mode,
        resume_ids: Vec::new(),
        activate_message,
    }) {
        Ok(app) => app,
        Err(e) => {
            log::error!("창 생성 실패: {e}");
            nwg::error_message(APP_NAME, &format!("창 생성 실패: {e}"));
            return;
        }
    };
    nwg::dispatch_thread_events();
    app.shutdown();
    drop(app);
    log::info!("{APP_NAME} 종료");
}
