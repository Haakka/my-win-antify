use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt};

use shared::DEFAULT_PIPE_NAME;

#[link(name = "kernel32")]
extern "system" {
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: isize,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

pub(crate) fn pipe_available() -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    let pipe_wide = wide(OsStr::new(DEFAULT_PIPE_NAME));
    (unsafe { WaitNamedPipeW(pipe_wide.as_ptr(), 20) != 0 })
        || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_BUSY)
}

pub(crate) fn ensure_core_service_running() -> Result<bool, String> {
    if pipe_available() {
        return Ok(false);
    }

    const SW_HIDE: i32 = 0;
    const LAUNCH_FAILED: &str = "启动后台服务失败或已取消管理员授权（UAC）。";
    let executable = std::env::current_exe()
        .map_err(|error| format!("{LAUNCH_FAILED} {error}"))?
        .with_file_name("core-service.exe");
    if !executable.is_file() {
        return Err("未找到同目录下的 core-service.exe，请重新解压完整运行包。".into());
    }
    let directory = executable
        .parent()
        .ok_or_else(|| LAUNCH_FAILED.to_owned())?;
    let executable = wide(executable.as_os_str());
    let directory = wide(directory.as_os_str());
    let operation = wide(OsStr::new("runas"));
    let arguments = format!("--parent-pid {}", std::process::id());
    let arguments = wide(OsStr::new(&arguments));

    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            executable.as_ptr(),
            arguments.as_ptr(),
            directory.as_ptr(),
            SW_HIDE,
        )
    };
    if result > 32 {
        Ok(true)
    } else {
        Err(LAUNCH_FAILED.into())
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}
