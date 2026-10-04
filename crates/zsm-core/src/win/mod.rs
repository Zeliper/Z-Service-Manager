//! All `unsafe` Win32 calls live under this module; only safe wrappers are exported.

mod console;
mod env;
mod job;
mod process;
mod window;

pub use console::{enable_ctrl_c, on_ctrl_event, send_ctrl_break_attached, CtrlEvent};
pub use env::{expand_env, local_time, LocalTime};
pub use job::Job;
pub use process::{spawn, spawn_lock, Process, SpawnOptions, Spawned, Stdio, Visibility};
pub use window::{post_close, set_visible, top_level_windows, WindowId};

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::Foundation::HANDLE;

fn owned(handle: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn io_err(e: windows::core::Error) -> std::io::Error {
    std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}
