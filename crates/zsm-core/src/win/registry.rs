use std::io;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

use super::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn check(err: WIN32_ERROR) -> io::Result<()> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(err.0 as i32))
    }
}

/// `HKCU\...\Run\<name>` value, if present.
pub fn run_entry(name: &str) -> Option<String> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    let mut size = 0u32;
    let first = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if first != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; size as usize / 2 + 1];
    let res = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if res != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn set_run_entry(name: &str, command: &str) -> io::Result<()> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    let data = wide(command);
    let mut hkey = HKEY::default();
    unsafe {
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        ))?;
        let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
        let res = check(RegSetValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            REG_SZ,
            Some(bytes),
        ));
        let _ = RegCloseKey(hkey);
        res
    }
}

pub fn remove_run_entry(name: &str) -> io::Result<()> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    let mut hkey = HKEY::default();
    unsafe {
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        ))?;
        let res = RegDeleteValueW(hkey, PCWSTR(value.as_ptr()));
        let _ = RegCloseKey(hkey);
        if res == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check(res)
        }
    }
}
