use std::io;
use std::os::windows::io::OwnedHandle;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, PostMessageW, RegisterWindowMessageW, ASFW_ANY, HWND_BROADCAST,
};

use super::{io_err, owned, wide};

/// Held for the lifetime of the first instance.
pub struct SingleInstance(#[allow(dead_code)] OwnedHandle);

/// `Ok(None)` when another instance already owns the named mutex.
pub fn acquire_single_instance(name: &str) -> io::Result<Option<SingleInstance>> {
    let name = wide(name);
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(io_err)?;
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let guard = SingleInstance(owned(handle));
    Ok((!exists).then_some(guard))
}

pub fn register_message(name: &str) -> u32 {
    let name = wide(name);
    unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
}

/// Lets the receiving instance take the foreground, then broadcasts `message`.
pub fn broadcast_message(message: u32) {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let _ = PostMessageW(Some(HWND_BROADCAST), message, WPARAM(0), LPARAM(0));
    }
}
