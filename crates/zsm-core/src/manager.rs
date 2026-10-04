use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use crate::config::Config;
use crate::supervisor::{Context, CtrlBreakHelper, Event, Notify, Service, StopMode};

pub struct ManagerOptions {
    pub log_root: std::path::PathBuf,
    pub notify: Option<Notify>,
    pub ctrl_break_helper: Option<CtrlBreakHelper>,
}

/// Owns every service supervisor, in config order.
pub struct Manager {
    ctx: Context,
    services: Vec<Service>,
}

impl Manager {
    pub fn new(opts: ManagerOptions) -> (Manager, Receiver<Event>) {
        let (events, rx) = crossbeam_channel::unbounded();
        let ctx = Context {
            log_root: opts.log_root,
            scrollback: 10_000,
            events,
            notify: opts.notify,
            ctrl_break_helper: opts.ctrl_break_helper,
        };
        (
            Manager {
                ctx,
                services: Vec::new(),
            },
            rx,
        )
    }

    /// Adds, updates and removes services so they match `config`.
    pub fn apply(&mut self, config: &Config) {
        self.ctx.scrollback = config.app.scrollback_lines;
        let mut old = std::mem::take(&mut self.services);
        for entry in &config.services {
            let id = &entry.config.id;
            match old.iter().position(|s| s.id() == id) {
                Some(i) => {
                    let svc = old.remove(i);
                    svc.update(entry.config.clone(), entry.error.clone());
                    svc.set_scrollback(config.app.scrollback_lines);
                    self.services.push(svc);
                }
                None => self.services.push(Service::spawn(
                    entry.config.clone(),
                    entry.error.clone(),
                    self.ctx.clone(),
                )),
            }
        }
        for svc in &old {
            svc.stop(StopMode::Normal);
        }
        std::thread::spawn(move || {
            wait_inactive(&old, Duration::from_secs(120));
            drop(old);
        });
    }

    pub fn services(&self) -> &[Service] {
        &self.services
    }

    pub fn get(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.id() == id)
    }

    pub fn start_autostart(&self) {
        for svc in self.services.iter().filter(|s| s.config().autostart) {
            svc.start();
        }
    }

    pub fn start_all(&self) {
        self.services.iter().for_each(Service::start);
    }

    pub fn stop_all(&self, mode: StopMode) {
        self.services.iter().for_each(|s| s.stop(mode));
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.services
            .iter()
            .filter(|s| s.status().state.is_active())
            .map(|s| s.id().to_owned())
            .collect()
    }

    /// Blocks until no service is starting, running or stopping.
    pub fn wait_all_inactive(&self, timeout: Duration) -> bool {
        wait_inactive(&self.services, timeout)
    }
}

fn wait_inactive(services: &[Service], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if services.iter().all(|s| !s.status().state.is_active()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
