//! All `unsafe` Win32 calls live under this module; only safe wrappers are exported.

mod console;
mod env;
mod instance;
mod job;
mod process;
mod registry;
mod richedit;
mod shell;
mod window;

pub use console::{enable_ctrl_c, on_ctrl_event, send_ctrl_break_attached, CtrlEvent};
pub use env::{expand_env, local_time, LocalTime};
pub use instance::{acquire_single_instance, broadcast_message, register_message, SingleInstance};
pub use job::Job;
pub use process::{spawn, spawn_lock, Process, SpawnOptions, Spawned, Stdio, Visibility};
pub use registry::{remove_run_entry, run_entry, set_run_entry};
pub use richedit::RichEdit;
pub use shell::{
    block_shutdown, bring_to_front, ctrl_down, cursor_pos, request_early_shutdown_notification,
    shell_open, unblock_shutdown,
};
pub use window::{
    dialog_code_message, post_close, set_menu_item_text, set_visible, top_level_windows, WindowId,
};

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
