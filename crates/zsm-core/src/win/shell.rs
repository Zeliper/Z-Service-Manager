use std::io;

use windows::core::PCWSTR;
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::Threading::SetProcessShutdownParameters;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNORMAL,
};

use super::{io_err, wide, WindowId};

/// Opens a file, folder or URL with its default handler.
pub fn shell_open(target: &str) -> io::Result<()> {
    let op = wide("open");
    let file = wide(target);
    let res = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if res.0 as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecute 실패 ({})",
            res.0 as isize
        )))
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

pub fn bring_to_front(window: WindowId) {
    unsafe {
        if IsIconic(window.hwnd()).as_bool() {
            let _ = ShowWindow(window.hwnd(), SW_RESTORE);
        }
        let _ = SetForegroundWindow(window.hwnd());
    }
}

/// Asks Windows to notify this process early at logoff/shutdown.
pub fn request_early_shutdown_notification() {
    unsafe {
        let _ = SetProcessShutdownParameters(0x3FF, 0);
    }
}

pub fn block_shutdown(window: WindowId, reason: &str) -> io::Result<()> {
    let reason = wide(reason);
    unsafe { ShutdownBlockReasonCreate(window.hwnd(), PCWSTR(reason.as_ptr())) }.map_err(io_err)
}

pub fn unblock_shutdown(window: WindowId) {
    unsafe {
        let _ = ShutdownBlockReasonDestroy(window.hwnd());
    }
}
