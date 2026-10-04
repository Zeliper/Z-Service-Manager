use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::win::LocalTime;

/// Daily plain-text log at `<dir>/YYYY-MM-DD.log`.
pub struct ServiceLog {
    dir: PathBuf,
    date: String,
    file: Option<BufWriter<File>>,
}

impl ServiceLog {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            date: String::new(),
            file: None,
        }
    }

    pub fn write(&mut self, time: &LocalTime, text: &str) {
        let date = time.date_string();
        if self.file.is_none() || date != self.date {
            self.flush();
            self.file = self.open(&date);
            self.date = date;
        }
        if let Some(f) = &mut self.file {
            if writeln!(f, "[{}] {}", time.time_string(), text).is_err() {
                self.file = None;
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
    }

    fn open(&self, date: &str) -> Option<BufWriter<File>> {
        std::fs::create_dir_all(&self.dir).ok()?;
        let path = self.dir.join(format!("{date}.log"));
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => Some(BufWriter::new(f)),
            Err(e) => {
                log::warn!("로그 파일 열기 실패 {}: {e}", path.display());
                None
            }
        }
    }
}

impl Drop for ServiceLog {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Deletes `*.log` files under each service directory older than `retention_days`.
pub fn purge_old_logs(root: &Path, retention_days: u32) {
    let Some(cutoff) =
        SystemTime::now().checked_sub(Duration::from_secs(retention_days as u64 * 86_400))
    else {
        return;
    };
    let Ok(services) = std::fs::read_dir(root) else {
        return;
    };
    for dir in services.flatten().filter(|d| d.path().is_dir()) {
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let old = file
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < cutoff);
            if old && path.extension().is_some_and(|e| e == "log") {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("오래된 로그 삭제 실패 {}: {e}", path.display());
                }
            }
        }
    }
}
