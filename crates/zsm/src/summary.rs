use zsm_core::supervisor::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayColor {
    Gray,
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub color: TrayColor,
    pub running: usize,
    pub total: usize,
}

/// Overall state from `(state, autostart)` per service.
/// Priority: red (failed/invalid) > yellow (transitional) > gray (nothing running) > green.
pub fn summarize(services: &[(State, bool)]) -> Summary {
    let running = services
        .iter()
        .filter(|(s, _)| *s == State::Running)
        .count();
    let any = |pred: fn(State) -> bool| services.iter().any(|(s, _)| pred(*s));
    let autostart_down = services
        .iter()
        .any(|(s, auto)| *auto && *s != State::Running);
    let color = if any(|s| matches!(s, State::Failed | State::Invalid)) {
        TrayColor::Red
    } else if any(|s| matches!(s, State::Starting | State::Stopping | State::Backoff)) {
        TrayColor::Yellow
    } else if running == 0 {
        TrayColor::Gray
    } else if autostart_down {
        TrayColor::Yellow
    } else {
        TrayColor::Green
    };
    Summary {
        color,
        running,
        total: services.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use State::*;

    #[test]
    fn colors() {
        assert_eq!(summarize(&[]).color, TrayColor::Gray);
        assert_eq!(summarize(&[(Stopped, true)]).color, TrayColor::Gray);
        assert_eq!(
            summarize(&[(Running, true), (Stopped, false)]).color,
            TrayColor::Green
        );
        assert_eq!(
            summarize(&[(Running, true), (Stopped, true)]).color,
            TrayColor::Yellow
        );
        assert_eq!(
            summarize(&[(Running, true), (Backoff, false)]).color,
            TrayColor::Yellow
        );
        assert_eq!(
            summarize(&[(Running, true), (Invalid, false)]).color,
            TrayColor::Red
        );
        let s = summarize(&[(Running, true), (Running, false), (Stopped, false)]);
        assert_eq!((s.running, s.total), (2, 3));
    }
}
