use std::time::Duration;

use zsm_core::supervisor::State;

pub fn state_label(state: State) -> &'static str {
    match state {
        State::Stopped => "중지됨",
        State::Starting => "시작 중",
        State::Running => "실행 중",
        State::Stopping => "중지 중",
        State::Crashed => "크래시",
        State::Backoff => "재시작 대기",
        State::Failed => "실패",
        State::Invalid => "설정 오류",
    }
}

pub fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    let (days, h, m, s) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    if days > 0 {
        format!("{days}d {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

pub fn memory(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{mb:.1} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(uptime(Duration::from_secs(3661)), "01:01:01");
        assert_eq!(uptime(Duration::from_secs(90_061)), "1d 01:01:01");
        assert_eq!(memory(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(memory(3 * 1024 * 1024 * 1024), "3.00 GB");
    }
}
