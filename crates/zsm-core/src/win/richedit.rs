use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    GetScrollInfo, SendMessageW, SB_BOTTOM, SB_VERT, SCROLLINFO, SIF_ALL, WM_COPY, WM_SETREDRAW,
    WM_USER, WM_VSCROLL,
};

use super::{wide, WindowId};

const EM_SETSEL: u32 = 0x00B1;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_EXGETSEL: u32 = WM_USER + 52;
const EM_EXSETSEL: u32 = WM_USER + 55;
const EM_SETCHARFORMAT: u32 = WM_USER + 68;
const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;
const SCF_SELECTION: usize = 0x0001;
const CFM_COLOR: u32 = 0x4000_0000;
const CFM_BOLD: u32 = 0x0000_0001;
const CFE_BOLD: u32 = 0x0000_0001;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CharRange {
    cp_min: i32,
    cp_max: i32,
}

#[repr(C)]
struct CharFormatW {
    cb_size: u32,
    dw_mask: u32,
    dw_effects: u32,
    y_height: i32,
    y_offset: i32,
    cr_text_color: u32,
    b_char_set: u8,
    b_pitch_and_family: u8,
    sz_face_name: [u16; 32],
}

/// Incremental output operations on a RichEdit control.
#[derive(Clone, Copy)]
pub struct RichEdit(pub WindowId);

impl RichEdit {
    fn send(&self, msg: u32, w: usize, l: isize) -> isize {
        unsafe { SendMessageW(self.0.hwnd(), msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
    }

    fn selection(&self) -> CharRange {
        let mut r = CharRange::default();
        self.send(EM_EXGETSEL, 0, &mut r as *mut _ as isize);
        r
    }

    fn set_selection(&self, r: CharRange) {
        self.send(EM_EXSETSEL, 0, &r as *const _ as isize);
    }

    pub fn has_selection(&self) -> bool {
        let r = self.selection();
        r.cp_max > r.cp_min
    }

    pub fn copy(&self) {
        self.send(WM_COPY, 0, 0);
    }

    pub fn is_at_bottom(&self) -> bool {
        let mut si = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        if unsafe { GetScrollInfo(self.0.hwnd(), SB_VERT, &mut si) }.is_err() {
            return true;
        }
        si.nPage == 0 || si.nPos + si.nPage as i32 >= si.nMax
    }

    fn batch(&self, f: impl FnOnce()) {
        let follow = self.is_at_bottom();
        let saved_sel = self.selection();
        let mut scroll = POINT::default();
        self.send(EM_GETSCROLLPOS, 0, &mut scroll as *mut _ as isize);
        self.send(WM_SETREDRAW, 0, 0);
        f();
        self.set_selection(saved_sel);
        if follow {
            self.send(WM_VSCROLL, SB_BOTTOM.0 as usize, 0);
        } else {
            self.send(EM_SETSCROLLPOS, 0, &scroll as *const _ as isize);
        }
        self.send(WM_SETREDRAW, 1, 0);
        unsafe {
            let _ = InvalidateRect(Some(self.0.hwnd()), None, true);
        }
    }

    /// Appends `(text, 0x00BBGGRR color, bold)` chunks at the end. Keeps the user's selection,
    /// and keeps the scroll position unless the view was already at the bottom.
    pub fn append(&self, chunks: &[(&str, u32, bool)]) {
        self.batch(|| {
            for &(text, rgb, bold) in chunks {
                // Positions past the end clamp to the end; (-1, -1) would only deselect.
                self.send(EM_SETSEL, i32::MAX as usize, i32::MAX as isize);
                self.apply_format(rgb, bold);
                let w = wide(text);
                self.send(EM_REPLACESEL, 0, PCWSTR(w.as_ptr()).0 as isize);
            }
        });
    }

    /// Removes the first `chars` UTF-16 units (a paragraph break counts as one).
    pub fn remove_prefix(&self, chars: usize) {
        if chars == 0 {
            return;
        }
        self.batch(|| {
            self.send(EM_SETSEL, 0, chars as isize);
            let empty = [0u16];
            self.send(EM_REPLACESEL, 0, empty.as_ptr() as isize);
        });
    }

    pub fn set_background(&self, rgb: u32) {
        self.send(EM_SETBKGNDCOLOR, 0, rgb as isize);
    }

    fn apply_format(&self, rgb: u32, bold: bool) {
        let cf = CharFormatW {
            cb_size: std::mem::size_of::<CharFormatW>() as u32,
            dw_mask: CFM_COLOR | CFM_BOLD,
            dw_effects: if bold { CFE_BOLD } else { 0 },
            y_height: 0,
            y_offset: 0,
            cr_text_color: rgb,
            b_char_set: 0,
            b_pitch_and_family: 0,
            sz_face_name: [0; 32],
        };
        self.send(EM_SETCHARFORMAT, SCF_SELECTION, &cf as *const _ as isize);
    }
}
