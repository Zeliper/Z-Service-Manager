use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub struct Metrics {
    sys: System,
    cpus: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            sys: System::new(),
            cpus: std::thread::available_parallelism().map_or(1, |n| n.get()) as f32,
        }
    }

    /// Total CPU% (0–100 across all cores) and resident memory in bytes for `pids`.
    pub fn sample(&mut self, pids: &[u32]) -> (f32, u64) {
        let pids: Vec<Pid> = pids.iter().map(|&p| Pid::from_u32(p)).collect();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        pids.iter()
            .filter_map(|p| self.sys.process(*p))
            .fold((0.0, 0), |(cpu, mem), p| {
                (cpu + p.cpu_usage() / self.cpus, mem + p.memory())
            })
    }
}
