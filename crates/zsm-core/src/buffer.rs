use std::collections::VecDeque;
use std::sync::Mutex;

use crate::logfile::ServiceLog;
use crate::output::{Span, StyledLine};
use crate::win::{local_time, LocalTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Output,
    Input,
    System,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub seq: u64,
    pub kind: LineKind,
    pub time: LocalTime,
    pub text: String,
    pub spans: Vec<Span>,
}

struct Ring {
    lines: VecDeque<Line>,
    next_seq: u64,
    capacity: usize,
}

/// Scrollback ring plus log file for one service; shared by the reader and supervisor threads.
pub struct OutputSink {
    ring: Mutex<Ring>,
    log: Mutex<ServiceLog>,
}

impl OutputSink {
    pub fn new(capacity: usize, log: ServiceLog) -> Self {
        Self {
            ring: Mutex::new(Ring {
                lines: VecDeque::new(),
                next_seq: 1,
                capacity: capacity.max(1),
            }),
            log: Mutex::new(log),
        }
    }

    pub fn push(&self, kind: LineKind, line: StyledLine) {
        self.push_all(kind, std::iter::once(line));
    }

    pub fn push_all(&self, kind: LineKind, lines: impl IntoIterator<Item = StyledLine>) {
        let time = local_time();
        let lines: Vec<StyledLine> = lines.into_iter().collect();
        if lines.is_empty() {
            return;
        }
        {
            let mut log = self.log.lock().unwrap();
            for l in &lines {
                log.write(&time, &l.text);
            }
            log.flush();
        }
        let mut ring = self.ring.lock().unwrap();
        for l in lines {
            let seq = ring.next_seq;
            ring.next_seq += 1;
            ring.lines.push_back(Line {
                seq,
                kind,
                time,
                text: l.text,
                spans: l.spans,
            });
        }
        while ring.lines.len() > ring.capacity {
            ring.lines.pop_front();
        }
    }

    pub fn system(&self, text: impl AsRef<str>) {
        self.push(
            LineKind::System,
            StyledLine::plain(format!("[zsm] {}", text.as_ref())),
        );
    }

    /// Lines with `seq > after`, oldest first.
    pub fn since(&self, after: u64) -> Vec<Line> {
        let ring = self.ring.lock().unwrap();
        let skip = ring.lines.partition_point(|l| l.seq <= after);
        ring.lines.iter().skip(skip).cloned().collect()
    }

    pub fn set_capacity(&self, capacity: usize) {
        let mut ring = self.ring.lock().unwrap();
        ring.capacity = capacity.max(1);
        while ring.lines.len() > ring.capacity {
            ring.lines.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_drops_oldest_and_since_is_incremental() {
        let dir = std::env::temp_dir().join(format!("zsm-buffer-{}", std::process::id()));
        let sink = OutputSink::new(3, ServiceLog::new(dir.clone()));
        for i in 0..5 {
            sink.push(LineKind::Output, StyledLine::plain(format!("l{i}")));
        }
        let all = sink.since(0);
        assert_eq!(
            all.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["l2", "l3", "l4"]
        );
        assert_eq!(sink.since(all[1].seq).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
