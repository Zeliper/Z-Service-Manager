use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Sender;
use native_windows_gui as nwg;
use zsm_core::update::{self, Release};
use zsm_core::VERSION;

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);

pub enum UpdateMsg {
    Checked {
        manual: bool,
        result: Result<Option<Release>, String>,
    },
    Downloaded(Result<PathBuf, String>),
    ServicesStopped(PathBuf),
}

fn send(tx: &Sender<UpdateMsg>, notice: nwg::NoticeSender, msg: UpdateMsg) {
    let _ = tx.send(msg);
    notice.notice();
}

pub fn check_now(tx: Sender<UpdateMsg>, notice: nwg::NoticeSender, manual: bool) {
    std::thread::spawn(move || {
        let result = update::check(VERSION);
        if let Err(e) = &result {
            log::warn!("업데이트 확인 실패: {e}");
        }
        send(&tx, notice, UpdateMsg::Checked { manual, result });
    });
}

/// Checks 30 s after start, then every `interval_hours`.
pub fn schedule(tx: Sender<UpdateMsg>, notice: nwg::NoticeSender, interval_hours: u64) {
    let interval = Duration::from_secs(interval_hours.max(1) * 3600);
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                let result = update::check(VERSION);
                if let Err(e) = &result {
                    log::warn!("업데이트 확인 실패: {e}");
                }
                send(
                    &tx,
                    notice,
                    UpdateMsg::Checked {
                        manual: false,
                        result,
                    },
                );
                std::thread::sleep(interval);
            }
        })
        .expect("spawn update scheduler");
}

pub fn download(tx: Sender<UpdateMsg>, notice: nwg::NoticeSender, release: Release) {
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join("ZServiceManager-update");
        let result = update::download_verified(&release, &dir);
        send(&tx, notice, UpdateMsg::Downloaded(result));
    });
}

pub fn wait_then_install(
    tx: Sender<UpdateMsg>,
    notice: nwg::NoticeSender,
    activity: zsm_core::manager::Activity,
    installer: PathBuf,
) {
    std::thread::spawn(move || {
        activity.wait_inactive(Duration::from_secs(3600));
        send(&tx, notice, UpdateMsg::ServicesStopped(installer));
    });
}
