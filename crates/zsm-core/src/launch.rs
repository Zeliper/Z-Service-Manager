use std::io::{self, Read, Write};
use std::path::Path;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::config::{IoMode, Kind, ServiceConfig};
use crate::win::{self, expand_env, Job, Process, SpawnOptions, Stdio, Visibility};

pub const PTY_COLS: u16 = 250;
pub const PTY_ROWS: u16 = 50;

pub struct Launched {
    pub process: Process,
    pub writer: Option<Box<dyn Write + Send>>,
    pub reader: Option<Box<dyn Read + Send>>,
    /// Keeps the pseudoconsole open; dropping it closes the PTY and ends the reader.
    pub pty: Option<PtyHandles>,
}

pub struct PtyHandles {
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

struct Resolved {
    program: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
}

fn resolve(cfg: &ServiceConfig) -> Resolved {
    let mut program = expand_env(cfg.command.trim());
    let mut cwd = Some(expand_env(cfg.working_dir.trim())).filter(|c| !c.is_empty());
    let path = Path::new(&program);
    if path.is_relative() {
        if let Some(dir) = &cwd {
            let candidate = Path::new(dir).join(path);
            if candidate.is_file() {
                program = candidate.to_string_lossy().into_owned();
            }
        }
    } else if cwd.is_none() {
        cwd = path.parent().map(|p| p.to_string_lossy().into_owned());
    }
    Resolved {
        program,
        args: cfg.args.iter().map(|a| expand_env(a)).collect(),
        cwd,
        env: cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), expand_env(v)))
            .collect(),
    }
}

pub fn launch(cfg: &ServiceConfig, job: &Job) -> io::Result<Launched> {
    let r = resolve(cfg);
    match (cfg.kind, cfg.io_mode) {
        (Kind::Console, IoMode::Pty) => launch_pty(&r, job),
        (Kind::Console, IoMode::Pipe) => launch_direct(&r, job, true),
        (Kind::Gui, _) => launch_direct(&r, job, false),
    }
}

fn launch_direct(r: &Resolved, job: &Job, console: bool) -> io::Result<Launched> {
    let opts = SpawnOptions {
        program: &r.program,
        args: &r.args,
        cwd: r.cwd.as_deref(),
        env: &r.env,
        stdio: if console {
            Stdio::Pipes
        } else {
            Stdio::Inherit
        },
        visibility: if console {
            Visibility::Normal
        } else {
            Visibility::Hidden
        },
        new_process_group: console,
        no_window: console,
    };
    let spawned = win::spawn(&opts, Some(job))?;
    Ok(Launched {
        process: spawned.process,
        writer: spawned.stdin.map(|f| Box::new(f) as Box<dyn Write + Send>),
        reader: spawned.stdout.map(|f| Box::new(f) as Box<dyn Read + Send>),
        pty: None,
    })
}

fn launch_pty(r: &Resolved, job: &Job) -> io::Result<Launched> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: PTY_ROWS,
            cols: PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(io::Error::other)?;
    let mut cmd = CommandBuilder::new(&r.program);
    cmd.args(&r.args);
    if let Some(cwd) = &r.cwd {
        cmd.cwd(cwd);
    }
    for (k, v) in &r.env {
        cmd.env(k, v);
    }
    // The child inherits our "ignore Ctrl+C" flag, which would make step 2 of the stop sequence a no-op.
    let _ = win::enable_ctrl_c();
    let child = {
        let _guard = win::spawn_lock();
        pair.slave
            .spawn_command(cmd)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e.to_string()))?
    };
    drop(pair.slave);
    let pid = child
        .process_id()
        .ok_or_else(|| io::Error::other("PTY 자식 PID 를 얻지 못함"))?;
    // ConPTY cannot create the child suspended, so it runs briefly before joining the job.
    let process = Process::open(pid)?;
    job.assign(&process)?;
    let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pair.master.take_writer().map_err(io::Error::other)?;
    Ok(Launched {
        process,
        writer: Some(writer),
        reader: Some(reader),
        pty: Some(PtyHandles {
            _child: child,
            _master: pair.master,
        }),
    })
}
