use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::{
        Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject},
        Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION},
    },
    UI::WindowsAndMessaging::{
        FindWindowW, MB_ICONERROR, MB_OK, MessageBoxW, SW_RESTORE, SetForegroundWindow, ShowWindow,
    },
};
use winreg::{RegKey, enums::*};

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub struct Instance {
    mutex: HANDLE,
    event: HANDLE,
}
impl Instance {
    pub fn acquire() -> io::Result<Option<Self>> {
        unsafe {
            let event = CreateEventW(ptr::null(), 0, 0, wide("Local\\serial-split.show").as_ptr());
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mutex = CreateMutexW(
                ptr::null(),
                0,
                wide("Local\\serial-split.instance").as_ptr(),
            );
            if mutex.is_null() {
                let e = io::Error::last_os_error();
                CloseHandle(event);
                return Err(e);
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                SetEvent(event);
                CloseHandle(event);
                CloseHandle(mutex);
                return Ok(None);
            }
            Ok(Some(Self { mutex, event }))
        }
    }
    pub fn show_requested(&self) -> bool {
        unsafe { WaitForSingleObject(self.event, 0) == 0 }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.event);
            CloseHandle(self.mutex);
        }
    }
}

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
fn startup_value(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}
pub fn autostart() -> io::Result<bool> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN) {
        Ok(k) => k,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    match key.get_value::<String, _>("serial-split") {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
pub fn set_autostart(enabled: bool, exe: &Path) -> io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN)?;
    if enabled {
        key.set_value("serial-split", &startup_value(exe))
    } else {
        match key.delete_value("serial-split") {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
pub fn refresh_autostart(exe: &Path) -> io::Result<bool> {
    let enabled = autostart()?;
    if enabled {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN)?;
        if key.get_value::<String, _>("serial-split")? != startup_value(exe) {
            set_autostart(true, exe)?;
        }
    }
    Ok(enabled)
}
pub fn timezone_offset() -> i32 {
    unsafe {
        let mut info: TIME_ZONE_INFORMATION = std::mem::zeroed();
        match GetTimeZoneInformation(&mut info) {
            2 => -(info.Bias + info.DaylightBias),
            1 => -(info.Bias + info.StandardBias),
            _ => -info.Bias,
        }
    }
}
pub fn fatal(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("serial-split 启动失败").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn foreground() {
    unsafe {
        let hwnd = FindWindowW(ptr::null(), wide("串口分路器").as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let ok = unsafe {
        MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
