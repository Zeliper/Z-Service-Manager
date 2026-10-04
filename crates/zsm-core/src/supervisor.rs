use std::collections::{HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use crate::buffer::{Line, LineKind, OutputSink};
use crate::config::{IoMode, Kind, RestartPolicy, ServiceConfig};
use crate::launch::{launch, PtyHandles};
use crate::logfile::ServiceLog;
use crate::metrics::Metrics;
use crate::output::{LineProcessor, StyledLine};
use crate::win::{self, Job, Process, WindowId};

const TICK: Duration = Duration::from_millis(100);
const METRICS_INTERVAL: Duration = Duration::from_secs(2);
const BACKOFF_MAX_SECS: u64 = 60;
const STABLE_RUN: Duration = Duration::from_secs(60);
const HIDE_PERIOD: Duration = Duration::from_secs(10);
const HIDE_INTERVAL: Duration = Duration::from_millis(500);
const SHUTDOWN_STEP_CAP: Duration = Duration::from_secs(20);
const KILL_GRACE: Duration = Duration::from_secs(10);
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
    Backoff,
    Failed,
    Invalid,
}

impl State {
    pub fn is_active(self) -> bool {
        matches!(self, State::Starting | State::Running | State::Stopping)
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub pid: Option<u32>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub started_at: Option<Instant>,
    pub restarts: u32,
    pub last_exit_code: Option<u32>,
    pub message: Option<String>,
    pub window_visible: bool,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: String,
    pub name: String,
    pub state: State,
    pub exit_code: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    Normal,
    /// Session end: every step's timeout is capped at 20 seconds.
    Shutdown,
}

/// External helper that sends CTRL_BREAK to a pipe-mode child: `<program> <args...> <pid>`.
#[derive(Clone, Debug)]
pub struct CtrlBreakHelper {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Context {
    pub log_root: PathBuf,
    pub scrollback: usize,
    pub events: Sender<Event>,
    pub notify: Option<Notify>,
    pub ctrl_break_helper: Option<CtrlBreakHelper>,
}

enum Command {
    Start,
    Stop(StopMode),
    Restart,
    Input(String),
    SetWindowVisible(bool),
    Update(Box<ServiceConfig>, Option<String>),
    SetScrollback(usize),
    Shutdown,
}

struct Shared {
    status: Mutex<Status>,
    config: Mutex<ServiceConfig>,
    sink: OutputSink,
}

/// Handle to one supervised service. Dropping it kills the process tree.
pub struct Service {
    id: String,
    shared: Arc<Shared>,
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Service {
    pub fn spawn(config: ServiceConfig, error: Option<String>, ctx: Context) -> Service {
        let id = config.id.clone();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status {
                state: if error.is_some() {
                    State::Invalid
                } else {
                    State::Stopped
                },
                pid: None,
                cpu_percent: 0.0,
                memory_bytes: 0,
                started_at: None,
                restarts: 0,
                last_exit_code: None,
                message: error.clone(),
                window_visible: false,
            }),
            config: Mutex::new(config.clone()),
            sink: OutputSink::new(ctx.scrollback, ServiceLog::new(ctx.log_root.join(&id))),
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let sup = Supervisor::new(config, error, ctx, shared.clone(), rx);
        let thread = std::thread::Builder::new()
            .name(format!("svc-{id}"))
            .spawn(move || sup.run())
            .expect("spawn supervisor thread");
        Service {
            id,
            shared,
            tx,
            thread: Some(thread),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn config(&self) -> ServiceConfig {
        self.shared.config.lock().unwrap().clone()
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn lines_since(&self, seq: u64) -> Vec<Line> {
        self.shared.sink.since(seq)
    }

    pub fn start(&self) {
        let _ = self.tx.send(Command::Start);
    }

    pub fn stop(&self, mode: StopMode) {
        let _ = self.tx.send(Command::Stop(mode));
    }

    pub fn restart(&self) {
        let _ = self.tx.send(Command::Restart);
    }

    pub fn send_line(&self, line: &str) {
        let _ = self.tx.send(Command::Input(line.to_owned()));
    }

    pub fn set_window_visible(&self, visible: bool) {
        let _ = self.tx.send(Command::SetWindowVisible(visible));
    }

    /// Takes effect on the next start if the service is running.
    pub fn update(&self, config: ServiceConfig, error: Option<String>) {
        let _ = self.tx.send(Command::Update(Box::new(config), error));
    }

    pub fn set_scrollback(&self, lines: usize) {
        let _ = self.tx.send(Command::SetScrollback(lines));
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

type SharedWriter = Arc<Mutex<Option<Box<dyn Write + Send>>>>;

struct Child {
    process: Process,
    job: Job,
    writer: SharedWriter,
    pty: Option<PtyHandles>,
    reader: Option<(JoinHandle<()>, Receiver<()>)>,
    started: Instant,
    hidden_windows: HashSet<WindowId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopPhase {
    Command,
    CtrlC,
    Close,
    Kill,
}

struct Supervisor {
    config: ServiceConfig,
    pending_config: Option<(ServiceConfig, Option<String>)>,
    invalid: Option<String>,
    ctx: Context,
    shared: Arc<Shared>,
    rx: Receiver<Command>,
    state: State,
    child: Option<Child>,
    stop: Option<(StopPhase, Instant)>,
    stop_cap: Option<Duration>,
    restart_after_stop: bool,
    backoff_until: Option<Instant>,
    backoff_exp: u32,
    crash_times: VecDeque<Instant>,
    metrics: Metrics,
    next_metrics: Instant,
    next_hide: Instant,
}

impl Supervisor {
    fn new(
        config: ServiceConfig,
        error: Option<String>,
        ctx: Context,
        shared: Arc<Shared>,
        rx: Receiver<Command>,
    ) -> Self {
        let state = if error.is_some() {
            State::Invalid
        } else {
            State::Stopped
        };
        Self {
            config,
            pending_config: None,
            invalid: error,
            ctx,
            shared,
            rx,
            state,
            child: None,
            stop: None,
            stop_cap: None,
            restart_after_stop: false,
            backoff_until: None,
            backoff_exp: 0,
            crash_times: VecDeque::new(),
            metrics: Metrics::new(),
            next_metrics: Instant::now(),
            next_hide: Instant::now(),
        }
    }

    fn sink(&self) -> &OutputSink {
        &self.shared.sink
    }

    fn run(mut self) {
        loop {
            let cmd = match self.rx.recv_timeout(TICK) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                if let Some(cmd) = cmd {
                    self.handle(cmd);
                }
                self.tick();
            }));
            if result.is_err() {
                log::error!("서비스 {} 감독 루프 패닉, 계속 진행", self.config.id);
            }
        }
        if let Some(child) = self.child.take() {
            let _ = child.job.terminate(1);
            self.release(child);
        }
    }

    fn set_state(&mut self, state: State, exit_code: Option<u32>) {
        self.state = state;
        self.shared.status.lock().unwrap().state = state;
        let _ = self.ctx.events.send(Event {
            id: self.config.id.clone(),
            name: self.config.display_name().to_owned(),
            state,
            exit_code,
        });
        if let Some(notify) = &self.ctx.notify {
            notify();
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Start => self.on_start_request(),
            Command::Stop(mode) => {
                self.restart_after_stop = false;
                match self.state {
                    State::Running | State::Starting => self.begin_stop(mode),
                    State::Stopping if mode == StopMode::Shutdown => self.cap_stop(),
                    State::Backoff => {
                        self.backoff_until = None;
                        self.sink().system("재시작 대기 취소");
                        self.set_state(State::Stopped, None);
                    }
                    _ => {}
                }
            }
            Command::Restart => match self.state {
                State::Running => {
                    self.restart_after_stop = true;
                    self.begin_stop(StopMode::Normal);
                }
                State::Stopping => self.restart_after_stop = true,
                _ => self.on_start_request(),
            },
            Command::Input(line) => self.send_input(&line),
            Command::SetWindowVisible(visible) => self.set_window_visible(visible),
            Command::Update(config, error) => {
                *self.shared.config.lock().unwrap() = (*config).clone();
                if self.child.is_some() {
                    self.pending_config = Some((*config, error));
                } else {
                    self.apply_config(*config, error);
                }
            }
            Command::SetScrollback(n) => self.shared.sink.set_capacity(n),
            Command::Shutdown => {}
        }
    }

    fn apply_config(&mut self, config: ServiceConfig, error: Option<String>) {
        self.config = config;
        self.invalid = error.clone();
        self.shared.status.lock().unwrap().message = error.clone();
        match (&error, self.state) {
            (Some(_), _) => {
                self.backoff_until = None;
                self.set_state(State::Invalid, None);
            }
            (None, State::Invalid) => self.set_state(State::Stopped, None),
            _ => {}
        }
    }

    fn on_start_request(&mut self) {
        match self.state {
            State::Invalid => {
                let msg = self.invalid.clone().unwrap_or_default();
                self.sink()
                    .system(format!("설정 오류로 시작할 수 없음: {msg}"));
            }
            State::Stopped | State::Crashed | State::Failed | State::Backoff => {
                self.crash_times.clear();
                self.backoff_exp = 0;
                self.backoff_until = None;
                self.start_now();
            }
            State::Starting | State::Running | State::Stopping => {}
        }
    }

    fn start_now(&mut self) {
        if let Some((config, error)) = self.pending_config.take() {
            self.apply_config(config, error);
        }
        if self.invalid.is_some() {
            return;
        }
        self.stop = None;
        self.set_state(State::Starting, None);
        let launched = Job::new().and_then(|job| launch(&self.config, &job).map(|l| (job, l)));
        let (job, launched) = match launched {
            Ok(v) => v,
            Err(e) => {
                self.sink().system(format!("시작 실패: {e}"));
                self.shared.status.lock().unwrap().message = Some(e.to_string());
                self.set_state(State::Crashed, None);
                if self.config.restart != RestartPolicy::Never {
                    self.schedule_restart(None);
                }
                return;
            }
        };
        let pid = launched.process.pid();
        let writer: SharedWriter = Arc::new(Mutex::new(launched.writer));
        let reader = launched
            .reader
            .map(|r| spawn_reader(&self.config.id, r, writer.clone(), self.shared.clone()));
        let now = Instant::now();
        self.child = Some(Child {
            process: launched.process,
            job,
            writer,
            pty: launched.pty,
            reader,
            started: now,
            hidden_windows: HashSet::new(),
        });
        {
            let mut st = self.shared.status.lock().unwrap();
            st.pid = Some(pid);
            st.started_at = Some(now);
            st.message = None;
            st.window_visible = false;
        }
        self.next_metrics = now;
        self.next_hide = now;
        self.sink().system(format!("시작됨 (PID {pid})"));
        self.set_state(State::Running, None);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let exited = self
            .child
            .as_ref()
            .and_then(|c| c.process.wait(Some(Duration::ZERO)).ok().flatten());
        if let Some(code) = exited {
            self.on_exit(code);
        }
        if let Some((phase, deadline)) = self.stop {
            if self.child.is_some() && now >= deadline {
                self.advance_stop(phase);
            }
        }
        if self.backoff_until.is_some_and(|t| now >= t) {
            self.backoff_until = None;
            self.shared.status.lock().unwrap().restarts += 1;
            self.start_now();
        }
        if self.child.is_some() && now >= self.next_metrics {
            self.next_metrics = now + METRICS_INTERVAL;
            self.sample_metrics();
        }
        if self.config.kind == Kind::Gui && now >= self.next_hide {
            self.next_hide = now + HIDE_INTERVAL;
            self.hide_new_windows(now);
        }
    }

    fn sample_metrics(&mut self) {
        let Some(child) = &self.child else { return };
        let pids = child.job.process_ids().unwrap_or_default();
        let (cpu, mem) = self.metrics.sample(&pids);
        let mut st = self.shared.status.lock().unwrap();
        st.cpu_percent = cpu;
        st.memory_bytes = mem;
    }

    fn on_exit(&mut self, code: u32) {
        let Some(child) = self.child.take() else {
            return;
        };
        let ran = child.started.elapsed();
        // Leftover descendants would keep the output pipe open; the service is over.
        let _ = child.job.terminate(1);
        self.release(child);
        {
            let mut st = self.shared.status.lock().unwrap();
            st.pid = None;
            st.cpu_percent = 0.0;
            st.memory_bytes = 0;
            st.started_at = None;
            st.last_exit_code = Some(code);
            st.window_visible = false;
        }
        self.sink().system(format!(
            "프로세스 종료 (종료 코드 {})",
            format_exit_code(code)
        ));

        if self.state == State::Stopping {
            self.stop = None;
            self.set_state(State::Stopped, Some(code));
            if std::mem::take(&mut self.restart_after_stop) {
                self.on_start_request();
            }
            return;
        }
        if code == 0 && self.config.restart != RestartPolicy::Always {
            self.set_state(State::Stopped, Some(code));
            return;
        }
        if code != 0 {
            self.set_state(State::Crashed, Some(code));
        }
        if self.config.restart != RestartPolicy::Never {
            self.schedule_restart(Some(ran));
        }
    }

    fn schedule_restart(&mut self, ran: Option<Duration>) {
        if ran.is_some_and(|r| r >= STABLE_RUN) {
            self.backoff_exp = 0;
        }
        let now = Instant::now();
        let window = Duration::from_secs(self.config.restart_window_secs);
        self.crash_times.retain(|t| now.duration_since(*t) < window);
        if self.crash_times.len() as u32 >= self.config.restart_max {
            self.sink().system(format!(
                "{}초 안에 재시작 {}회 초과, 중단",
                self.config.restart_window_secs, self.config.restart_max
            ));
            self.set_state(State::Failed, None);
            return;
        }
        self.crash_times.push_back(now);
        let delay = (1u64 << self.backoff_exp.min(6)).min(BACKOFF_MAX_SECS);
        self.backoff_exp += 1;
        self.backoff_until = Some(now + Duration::from_secs(delay));
        self.sink().system(format!(
            "{delay}초 후 재시작 ({}/{})",
            self.crash_times.len(),
            self.config.restart_max
        ));
        self.set_state(State::Backoff, None);
    }

    fn release(&mut self, child: Child) {
        child.writer.lock().unwrap().take();
        drop(child.pty);
        if let Some((handle, done)) = child.reader {
            if done.recv_timeout(READER_JOIN_TIMEOUT).is_ok() {
                let _ = handle.join();
            } else {
                log::warn!("서비스 {} 출력 리더가 제때 끝나지 않음", self.config.id);
            }
        }
    }

    fn step_timeout(&self, secs: u64) -> Duration {
        let t = Duration::from_secs(secs);
        self.stop_cap.map_or(t, |cap| t.min(cap))
    }

    fn begin_stop(&mut self, mode: StopMode) {
        if self.child.is_none() {
            return;
        }
        self.stop_cap = (mode == StopMode::Shutdown).then_some(SHUTDOWN_STEP_CAP);
        self.set_state(State::Stopping, None);
        let first = match self.config.kind {
            Kind::Gui => StopPhase::Close,
            Kind::Console if self.config.stop_command.trim().is_empty() => StopPhase::CtrlC,
            Kind::Console => StopPhase::Command,
        };
        self.enter_stop(first);
    }

    fn cap_stop(&mut self) {
        self.stop_cap = Some(SHUTDOWN_STEP_CAP);
        if let Some((phase, deadline)) = self.stop {
            let capped = Instant::now() + SHUTDOWN_STEP_CAP;
            self.stop = Some((phase, deadline.min(capped)));
        }
    }

    fn advance_stop(&mut self, phase: StopPhase) {
        match phase {
            StopPhase::Command => self.enter_stop(StopPhase::CtrlC),
            StopPhase::CtrlC | StopPhase::Close => self.enter_stop(StopPhase::Kill),
            StopPhase::Kill => {
                self.sink().system("강제 종료 후에도 프로세스가 남아 있음");
                self.stop = Some((StopPhase::Kill, Instant::now() + KILL_GRACE));
            }
        }
    }

    fn enter_stop(&mut self, phase: StopPhase) {
        let now = Instant::now();
        match phase {
            StopPhase::Command => {
                let cmd = self.config.stop_command.trim().to_owned();
                let t = self.step_timeout(self.config.stop_timeout_secs);
                self.write_line(&cmd);
                self.sink()
                    .system(format!("중지 명령 전송: {cmd} ({}초 대기)", t.as_secs()));
                self.stop = Some((phase, now + t));
            }
            StopPhase::CtrlC => {
                let t = self.step_timeout(self.config.ctrl_c_timeout_secs);
                let sent = match self.config.io_mode {
                    IoMode::Pty => self.write_raw(b"\x03").map(|_| "Ctrl+C"),
                    IoMode::Pipe => self.send_ctrl_break().map(|_| "Ctrl+Break"),
                };
                match sent {
                    Ok(what) => {
                        self.sink()
                            .system(format!("{what} 전송 ({}초 대기)", t.as_secs()));
                        self.stop = Some((phase, now + t));
                    }
                    Err(e) => {
                        self.sink()
                            .system(format!("Ctrl 이벤트 전송 실패, 건너뜀: {e}"));
                        self.enter_stop(StopPhase::Kill);
                    }
                }
            }
            StopPhase::Close => {
                let t = self.step_timeout(self.config.stop_timeout_secs);
                let windows = self.child_windows();
                for w in &windows {
                    win::post_close(*w);
                }
                self.sink().system(format!(
                    "WM_CLOSE 전송 (창 {}개, {}초 대기)",
                    windows.len(),
                    t.as_secs()
                ));
                self.stop = Some((phase, now + t));
            }
            StopPhase::Kill => {
                self.sink().system("강제 종료 (TerminateJobObject)");
                if let Some(child) = &self.child {
                    if let Err(e) = child.job.terminate(1) {
                        self.sink().system(format!("강제 종료 실패: {e}"));
                    }
                }
                self.stop = Some((phase, now + KILL_GRACE));
            }
        }
    }

    fn send_ctrl_break(&self) -> io::Result<()> {
        let helper = self
            .ctx
            .ctrl_break_helper
            .as_ref()
            .ok_or_else(|| io::Error::other("Ctrl+Break 헬퍼가 설정되지 않음"))?;
        let pid = self.child.as_ref().map(|c| c.process.pid()).unwrap_or(0);
        run_ctrl_break_helper(helper, pid)
    }

    fn write_raw(&self, bytes: &[u8]) -> io::Result<()> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("실행 중이 아님"))?;
        let mut writer = child.writer.lock().unwrap();
        let w = writer
            .as_mut()
            .ok_or_else(|| io::Error::other("입력이 연결되지 않음"))?;
        w.write_all(bytes)?;
        w.flush()
    }

    fn write_line(&self, line: &str) {
        let ending = match self.config.io_mode {
            IoMode::Pty => "\r",
            IoMode::Pipe => "\r\n",
        };
        if let Err(e) = self.write_raw(format!("{line}{ending}").as_bytes()) {
            self.sink().system(format!("입력 전송 실패: {e}"));
        }
    }

    fn send_input(&mut self, line: &str) {
        if self.config.kind != Kind::Console || self.child.is_none() {
            self.sink().system("실행 중이 아니라 입력을 보낼 수 없음");
            return;
        }
        if self.config.io_mode == IoMode::Pipe {
            self.sink()
                .push(LineKind::Input, StyledLine::plain(format!("> {line}")));
        }
        self.write_line(line);
    }

    fn child_windows(&self) -> Vec<WindowId> {
        let Some(child) = &self.child else {
            return Vec::new();
        };
        let pids = child.job.process_ids().unwrap_or_default();
        win::top_level_windows(&pids)
    }

    fn hide_new_windows(&mut self, now: Instant) {
        let visible_requested = self.shared.status.lock().unwrap().window_visible;
        let in_period = self
            .child
            .as_ref()
            .is_some_and(|c| now.duration_since(c.started) < HIDE_PERIOD);
        if !in_period || visible_requested {
            return;
        }
        let windows = self.child_windows();
        let Some(child) = &mut self.child else { return };
        for w in windows.into_iter().filter(|w| w.is_visible()) {
            win::set_visible(w, false);
            child.hidden_windows.insert(w);
        }
    }

    fn set_window_visible(&mut self, visible: bool) {
        if self.config.kind != Kind::Gui || self.child.is_none() {
            return;
        }
        let windows = self.child_windows();
        let Some(child) = &mut self.child else { return };
        let mut count = 0;
        for w in windows {
            if visible && (child.hidden_windows.contains(&w) || w.is_main()) {
                win::set_visible(w, true);
                count += 1;
            } else if !visible && w.is_visible() {
                win::set_visible(w, false);
                child.hidden_windows.insert(w);
                count += 1;
            }
        }
        self.shared.status.lock().unwrap().window_visible = visible;
        let what = if visible { "보이기" } else { "숨기기" };
        self.sink().system(format!("창 {what} (창 {count}개)"));
    }
}

fn format_exit_code(code: u32) -> String {
    if code > 0xFFFF {
        format!("0x{code:08X}")
    } else {
        code.to_string()
    }
}

fn spawn_reader(
    id: &str,
    mut reader: Box<dyn Read + Send>,
    writer: SharedWriter,
    shared: Arc<Shared>,
) -> (JoinHandle<()>, Receiver<()>) {
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    let handle = std::thread::Builder::new()
        .name(format!("out-{id}"))
        .spawn(move || {
            let mut processor = LineProcessor::new();
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let feed = processor.feed(&buf[..n]);
                if !feed.reply.is_empty() {
                    if let Some(w) = writer.lock().unwrap().as_mut() {
                        let _ = w.write_all(&feed.reply).and_then(|_| w.flush());
                    }
                }
                shared.sink.push_all(LineKind::Output, feed.lines);
            }
            if let Some(line) = processor.finish() {
                shared.sink.push(LineKind::Output, line);
            }
            let _ = done_tx.send(());
        })
        .expect("spawn reader thread");
    (handle, done_rx)
}

fn run_ctrl_break_helper(helper: &CtrlBreakHelper, pid: u32) -> io::Result<()> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let mut cmd = std::process::Command::new(&helper.program);
    cmd.args(&helper.args)
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS);
    let mut child = {
        let _guard = win::spawn_lock();
        cmd.spawn()?
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("헬퍼 종료 코드 {status}")))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "헬퍼 응답 없음"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
