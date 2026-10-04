use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, SetForegroundWindow, ShowWindow, GWL_STYLE, GW_OWNER, SW_HIDE,
    SW_SHOW, WM_CLOSE, WS_CAPTION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(isize);

impl WindowId {
    pub fn from_raw(hwnd: isize) -> WindowId {
        WindowId(hwnd)
    }

    pub(super) fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }

    pub fn is_visible(self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()) }.as_bool()
    }

    /// Titled window with a caption: what a user would call the app window.
    pub fn is_main(self) -> bool {
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd(), GWL_STYLE) as u32;
            style & WS_CAPTION.0 == WS_CAPTION.0 && GetWindowTextLengthW(self.hwnd()) > 0
        }
    }
}

struct EnumState<'a> {
    pids: &'a [u32],
    found: Vec<WindowId>,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut EnumState);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let unowned = GetWindow(hwnd, GW_OWNER).map_or(true, |h| h.is_invalid());
    if unowned && state.pids.contains(&pid) {
        state.found.push(WindowId(hwnd.0 as isize));
    }
    BOOL::from(true)
}

/// Unowned top-level windows belonging to any of `pids`.
pub fn top_level_windows(pids: &[u32]) -> Vec<WindowId> {
    let mut state = EnumState {
        pids,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut state as *mut _ as isize));
    }
    state.found
}

pub fn set_visible(window: WindowId, visible: bool) {
    unsafe {
        let _ = ShowWindow(window.hwnd(), if visible { SW_SHOW } else { SW_HIDE });
        if visible {
            let _ = SetForegroundWindow(window.hwnd());
        }
    }
}

pub fn post_close(window: WindowId) {
    unsafe {
        let _ = PostMessageW(Some(window.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// For `WM_GETDLGCODE`: the `(message, wparam)` of the MSG that `lparam` points to, if any.
pub fn dialog_code_message(lparam: isize) -> Option<(u32, usize)> {
    if lparam == 0 {
        return None;
    }
    let msg = unsafe { &*(lparam as *const windows::Win32::UI::WindowsAndMessaging::MSG) };
    Some((msg.message, msg.wParam.0))
}

/// Renames the menu item with command `id` in menu `hmenu`.
pub fn set_menu_item_text(hmenu: isize, id: u32, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetMenuItemInfoW, HMENU, MENUITEMINFOW, MIIM_STRING,
    };
    let mut wide = super::wide(text);
    let info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: windows::core::PWSTR(wide.as_mut_ptr()),
        ..Default::default()
    };
    unsafe {
        let _ = SetMenuItemInfoW(HMENU(hmenu as *mut _), id, false, &info);
    }
}
