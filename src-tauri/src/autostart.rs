use std::{
    ffi::{c_void, OsStr},
    io,
    os::windows::ffi::OsStrExt,
    ptr,
};

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        key: isize,
        subkey: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        access: u32,
        security: *const c_void,
        opened: *mut isize,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        key: isize,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        bytes: u32,
    ) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

struct RegistryKey(isize);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

pub(crate) fn sync_launch_at_login(enabled: bool) -> Result<(), String> {
    let command = if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| format!("Could not locate the MyWinAntify executable: {error}"))?;
        Some(launch_command(executable.as_os_str()))
    } else {
        None
    };
    let subkey: Vec<_> = RUN_KEY.encode_utf16().chain([0]).collect();
    let name: Vec<_> = "MyWinAntify".encode_utf16().chain([0]).collect();
    let mut key = 0;
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if result != 0 {
        return Err(registry_error("open", result));
    }
    let key = RegistryKey(key);
    if let Some(command) = command {
        let result = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                std::mem::size_of_val(command.as_slice()) as u32,
            )
        };
        if result != 0 {
            return Err(registry_error("set MyWinAntify in", result));
        }
    } else {
        let result = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if result != 0 && result != ERROR_FILE_NOT_FOUND {
            return Err(registry_error("remove MyWinAntify from", result));
        }
    }
    Ok(())
}

fn launch_command(executable: &OsStr) -> Vec<u16> {
    [u16::from(b'"')]
        .into_iter()
        .chain(executable.encode_wide())
        .chain([u16::from(b'"'), 0])
        .collect()
}

fn registry_error(action: &str, code: i32) -> String {
    format!(
        "Could not {action} HKCU\\{RUN_KEY}: {}",
        io::Error::from_raw_os_error(code)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_the_full_executable_path_and_terminates_the_registry_string() {
        let path = OsStr::new(r"C:\Program Files\测试\MyWinAntify.exe");
        let expected: Vec<_> = "\"C:\\Program Files\\测试\\MyWinAntify.exe\"\0"
            .encode_utf16()
            .collect();
        assert_eq!(launch_command(path), expected);
    }
}
