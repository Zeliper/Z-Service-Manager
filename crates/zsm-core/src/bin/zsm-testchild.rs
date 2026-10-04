//! Console program driven by the zsm-core tests.
//!
//! Flags:
//!   --tick-ms N        print `tick <n>` every N ms
//!   --ignore-stop      answer `/stop` with `ignoring stop` instead of exiting
//!   --ignore-ctrl      swallow Ctrl+C / Ctrl+Break
//!   --crash-after-ms N exit with code 1 after N ms
//!   --exit-code N      exit immediately with code N (after `ready`)
//!   --progress         print a `\r` progress bar
//!   --color            print an ANSI-colored line
//!   --spawn-child      start a grandchild (`--tick-ms 1000`) and print its pid
//!   --ctrl-break PID   helper mode: send CTRL_BREAK to PID's console and exit
//!
//! stdin: `/stop` exits 0, `crash` exits 1, anything else is echoed as `echo: <line>`.

use std::io::{BufRead, Write};
use std::time::Duration;

use zsm_core::win::{self, CtrlEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };

    if let Some(pid) = value("--ctrl-break") {
        std::process::exit(match win::send_ctrl_break_attached(pid as u32) {
            Ok(()) => 0,
            Err(_) => 1,
        });
    }

    if flag("--ignore-ctrl") {
        win::on_ctrl_event(|e| {
            println!("ctrl ignored {e:?}");
            true
        })
        .expect("install ctrl handler");
    } else {
        win::on_ctrl_event(|e| {
            println!("ctrl received {e:?}");
            let _ = std::io::stdout().flush();
            std::process::exit(if e == CtrlEvent::Break { 4 } else { 3 });
        })
        .expect("install ctrl handler");
    }

    println!("ready pid={}", std::process::id());

    if let Some(code) = value("--exit-code") {
        std::process::exit(code as i32);
    }
    if flag("--color") {
        println!("\x1b[31mred\x1b[0m plain");
    }
    if flag("--progress") {
        for p in [0, 50, 100] {
            print!("\rprogress {p}%");
            let _ = std::io::stdout().flush();
            std::thread::sleep(Duration::from_millis(30));
        }
        println!();
    }
    if flag("--spawn-child") {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--tick-ms", "1000"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn grandchild");
        println!("grandchild pid={}", child.id());
        std::thread::spawn(move || child.wait());
    }
    if let Some(ms) = value("--crash-after-ms") {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            println!("crashing");
            let _ = std::io::stdout().flush();
            std::process::exit(1);
        });
    }
    if let Some(ms) = value("--tick-ms") {
        std::thread::spawn(move || {
            let mut n = 0u64;
            loop {
                println!("tick {n}");
                n += 1;
                std::thread::sleep(Duration::from_millis(ms));
            }
        });
    }

    let ignore_stop = flag("--ignore-stop");
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        match line.trim() {
            "/stop" if ignore_stop => println!("ignoring stop"),
            "/stop" => {
                println!("stopping");
                std::process::exit(0);
            }
            "crash" => {
                println!("crashing");
                std::process::exit(1);
            }
            other => println!("echo: {other}"),
        }
    }
    // stdin closed: keep running like a server would until stopped.
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
