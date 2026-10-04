use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use zsm_core::buffer::LineKind;
use zsm_core::config::{IoMode, RestartPolicy, ServiceConfig};
use zsm_core::supervisor::{Context, CtrlBreakHelper, Event, Service, State, StopMode};
use zsm_core::win::Process;

const TESTCHILD: &str = env!("CARGO_BIN_EXE_zsm-testchild");
const WAIT: Duration = Duration::from_secs(15);

struct Fixture {
    service: Service,
    events: Receiver<Event>,
    log_root: PathBuf,
}

impl Fixture {
    fn new(
        name: &str,
        mode: IoMode,
        args: &[&str],
        tweak: impl FnOnce(&mut ServiceConfig),
    ) -> Self {
        let log_root =
            std::env::temp_dir().join(format!("zsm-test-{name}-{:?}-{}", mode, std::process::id()));
        let _ = std::fs::remove_dir_all(&log_root);
        let mut cfg = ServiceConfig::new(name, TESTCHILD);
        cfg.io_mode = mode;
        cfg.args = args.iter().map(|s| s.to_string()).collect();
        cfg.stop_command = "/stop".into();
        cfg.restart = RestartPolicy::Never;
        tweak(&mut cfg);
        let (tx, events) = crossbeam_channel::unbounded();
        let ctx = Context {
            log_root: log_root.clone(),
            scrollback: 1000,
            events: tx,
            notify: None,
            ctrl_break_helper: Some(CtrlBreakHelper {
                program: TESTCHILD.into(),
                args: vec!["--ctrl-break".into()],
            }),
        };
        let service = Service::spawn(cfg, None, ctx);
        Fixture {
            service,
            events,
            log_root,
        }
    }

    fn texts(&self) -> Vec<String> {
        self.service
            .lines_since(0)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    fn wait_line(&self, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(l) = self.texts().into_iter().find(|l| pred(l)) {
                return l;
            }
            assert!(
                Instant::now() < deadline,
                "line not found; got:\n{}",
                self.texts().join("\n")
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_state(&self, state: State) {
        let deadline = Instant::now() + WAIT;
        while self.service.status().state != state {
            assert!(
                Instant::now() < deadline,
                "state {:?} not reached (now {:?}); output:\n{}",
                state,
                self.service.status().state,
                self.texts().join("\n")
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn states(&self) -> Vec<State> {
        self.events.try_iter().map(|e| e.state).collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.log_root);
    }
}

fn pid_from(line: &str) -> u32 {
    line.rsplit('=').next().unwrap().trim().parse().unwrap()
}

fn is_alive(pid: u32) -> bool {
    Process::open(pid).is_ok_and(|p| p.is_running())
}

fn echo_and_stop(mode: IoMode) {
    let f = Fixture::new("echo", mode, &[], |_| {});
    f.service.start();
    f.wait_line(|l| l.starts_with("ready pid="));
    f.service.send_line("hello world");
    f.wait_line(|l| l == "echo: hello world");
    f.service.stop(StopMode::Normal);
    f.wait_state(State::Stopped);
    let texts = f.texts();
    assert!(texts.iter().any(|l| l == "stopping"), "{texts:#?}");
    assert!(!texts.iter().any(|l| l.contains("Ctrl")), "{texts:#?}");
    assert_eq!(f.service.status().last_exit_code, Some(0));
    if mode == IoMode::Pipe {
        let inputs: Vec<_> = f
            .service
            .lines_since(0)
            .into_iter()
            .filter(|l| l.kind == LineKind::Input)
            .map(|l| l.text)
            .collect();
        assert_eq!(inputs, ["> hello world"]);
    }
}

#[test]
fn pty_output_echo_and_stop_command() {
    echo_and_stop(IoMode::Pty);
}

#[test]
fn pipe_output_echo_and_stop_command() {
    echo_and_stop(IoMode::Pipe);
}

fn escalates_to_kill(mode: IoMode) {
    let f = Fixture::new("kill", mode, &["--ignore-stop", "--ignore-ctrl"], |c| {
        c.stop_timeout_secs = 1;
        c.ctrl_c_timeout_secs = 1;
    });
    f.service.start();
    f.wait_line(|l| l.starts_with("ready pid="));
    f.service.stop(StopMode::Normal);
    f.wait_state(State::Stopped);
    let texts = f.texts().join("\n");
    for expected in [
        "[zsm] 중지 명령 전송: /stop",
        "ignoring stop",
        "전송 (1초 대기)",
        "ctrl ignored",
        "[zsm] 강제 종료 (TerminateJobObject)",
    ] {
        assert!(
            texts.contains(expected),
            "missing `{expected}` in:\n{texts}"
        );
    }
}

#[test]
fn pty_stop_escalates_through_ctrl_c_to_kill() {
    escalates_to_kill(IoMode::Pty);
}

#[test]
fn pipe_stop_escalates_through_ctrl_break_to_kill() {
    escalates_to_kill(IoMode::Pipe);
}

fn ctrl_event_stops(mode: IoMode, expected: &str, code: u32) {
    let f = Fixture::new("ctrl", mode, &["--ignore-stop"], |c| {
        c.stop_timeout_secs = 1;
        c.ctrl_c_timeout_secs = 10;
    });
    f.service.start();
    f.wait_line(|l| l.starts_with("ready pid="));
    f.service.stop(StopMode::Normal);
    f.wait_state(State::Stopped);
    let texts = f.texts().join("\n");
    assert!(
        texts.contains(expected),
        "missing `{expected}` in:\n{texts}"
    );
    assert!(!texts.contains("강제 종료"), "{texts}");
    assert_eq!(f.service.status().last_exit_code, Some(code));
}

#[test]
fn pty_ctrl_c_stops_child() {
    ctrl_event_stops(IoMode::Pty, "ctrl received C", 3);
}

#[test]
fn pipe_ctrl_break_stops_child() {
    ctrl_event_stops(IoMode::Pipe, "ctrl received Break", 4);
}

#[test]
fn crash_restarts_with_backoff_then_fails() {
    let f = Fixture::new("crash", IoMode::Pipe, &["--crash-after-ms", "100"], |c| {
        c.restart = RestartPolicy::OnFailure;
        c.restart_max = 2;
    });
    let started = Instant::now();
    f.service.start();
    f.wait_state(State::Failed);
    let elapsed = started.elapsed();
    let states = f.states();
    use State::*;
    assert_eq!(
        states,
        [
            Starting, Running, Crashed, Backoff, Starting, Running, Crashed, Backoff, Starting,
            Running, Crashed, Failed
        ]
    );
    assert_eq!(f.service.status().restarts, 2);
    assert!(
        elapsed >= Duration::from_secs(3),
        "backoff 1s + 2s, got {elapsed:?}"
    );
    let texts = f.texts().join("\n");
    assert!(texts.contains("1초 후 재시작 (1/2)"), "{texts}");
    assert!(texts.contains("2초 후 재시작 (2/2)"), "{texts}");
}

#[test]
fn crash_via_input_restarts() {
    let f = Fixture::new("crashcmd", IoMode::Pty, &[], |c| {
        c.restart = RestartPolicy::OnFailure;
    });
    f.service.start();
    f.wait_line(|l| l.starts_with("ready pid="));
    f.service.send_line("crash");
    f.wait_line(|l| l.contains("1초 후 재시작"));
    f.wait_state(State::Running);
    assert_eq!(f.service.status().restarts, 1);
}

fn drop_kills_tree(mode: IoMode) {
    let f = Fixture::new("tree", mode, &["--spawn-child"], |_| {});
    f.service.start();
    let child = pid_from(&f.wait_line(|l| l.starts_with("ready pid=")));
    let grandchild = pid_from(&f.wait_line(|l| l.starts_with("grandchild pid=")));
    assert!(is_alive(child) && is_alive(grandchild));
    drop(f);
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_alive(child) || is_alive(grandchild) {
        assert!(Instant::now() < deadline, "process tree survived drop");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn pty_drop_kills_child_and_grandchild() {
    drop_kills_tree(IoMode::Pty);
}

#[test]
fn pipe_drop_kills_child_and_grandchild() {
    drop_kills_tree(IoMode::Pipe);
}

fn progress_and_ansi(mode: IoMode) {
    let f = Fixture::new("fmt", mode, &["--progress", "--color"], |_| {});
    f.service.start();
    f.wait_line(|l| l == "progress 100%");
    let red = f
        .service
        .lines_since(0)
        .into_iter()
        .find(|l| l.text == "red plain")
        .expect("color line");
    assert!(!red.spans.is_empty(), "SGR span kept: {red:?}");
    assert!(!f.texts().iter().any(|l| l.contains("progress 0%")));
    f.service.stop(StopMode::Normal);
    f.wait_state(State::Stopped);

    let dir = f.log_root.join("fmt");
    let log: String = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(log.contains("] red plain\n"), "{log}");
    assert!(log.contains("] progress 100%\n"), "{log}");
    assert!(!log.contains('\x1b'), "escape leaked into log:\n{log:?}");
    assert!(!log.contains('\r'), "CR leaked into log:\n{log:?}");
}

#[test]
fn pty_progress_merges_and_ansi_stripped_in_log() {
    progress_and_ansi(IoMode::Pty);
}

#[test]
fn pipe_progress_merges_and_ansi_stripped_in_log() {
    progress_and_ansi(IoMode::Pipe);
}
