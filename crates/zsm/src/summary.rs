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

/// Which per-service actions make sense right now (shared by buttons and the context menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allowed {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
    pub window: bool,
    pub input: bool,
}

pub fn allowed(state: Option<State>, gui: bool) -> Allowed {
    use State::*;
    Allowed {
        start: matches!(state, Some(Stopped | Crashed | Failed | Backoff)),
        stop: matches!(state, Some(Starting | Running | Backoff)),
        restart: matches!(state, Some(Running | Stopped | Crashed | Failed | Backoff)),
        window: gui && state == Some(Running),
        input: !gui && matches!(state, Some(Running | Stopping)),
    }
}

#[cfg(test)]
mod allowed_tests {
    use super::*;

    #[test]
    fn actions_follow_state() {
        assert_eq!(allowed(None, false), Allowed::default());
        let running = allowed(Some(State::Running), false);
        assert!(
            !running.start && running.stop && running.restart && running.input && !running.window
        );
        assert!(allowed(Some(State::Running), true).window);
        let invalid = allowed(Some(State::Invalid), false);
        assert!(!invalid.start && !invalid.stop && !invalid.restart);
    }
}
