use std::io;
use std::sync::OnceLock;
use windows::core::BOOL;
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler, CTRL_BREAK_EVENT,
    CTRL_C_EVENT,
};

use super::io_err;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlEvent {
    C,
    Break,
    Other(u32),
}

/// Attaches to the console of `pid` and sends CTRL_BREAK to its process group.
/// Must run in a process without a console of its own (helper process).
pub fn send_ctrl_break_attached(pid: u32) -> io::Result<()> {
    unsafe {
        let _ = FreeConsole();
        AttachConsole(pid).map_err(io_err)?;
        SetConsoleCtrlHandler(None, true).map_err(io_err)?;
        let res = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid).map_err(io_err);
        let _ = FreeConsole();
        res
    }
}

static CTRL_HANDLER: OnceLock<fn(CtrlEvent) -> bool> = OnceLock::new();

unsafe extern "system" fn trampoline(ctrl: u32) -> BOOL {
    let event = match ctrl {
        CTRL_C_EVENT => CtrlEvent::C,
        CTRL_BREAK_EVENT => CtrlEvent::Break,
        other => CtrlEvent::Other(other),
    };
    BOOL::from(CTRL_HANDLER.get().is_some_and(|h| h(event)))
}

/// Installs a process-wide console control handler. Returning `true` swallows the event.
pub fn on_ctrl_event(handler: fn(CtrlEvent) -> bool) -> io::Result<()> {
    let _ = CTRL_HANDLER.set(handler);
    unsafe { SetConsoleCtrlHandler(Some(trampoline), true) }.map_err(io_err)
}

/// Clears an inherited "ignore Ctrl+C" flag (set by CREATE_NEW_PROCESS_GROUP or a parent
/// calling `SetConsoleCtrlHandler(NULL, TRUE)`); children inherit the cleared state.
pub fn enable_ctrl_c() -> io::Result<()> {
    unsafe { SetConsoleCtrlHandler(None, false) }.map_err(io_err)
}
