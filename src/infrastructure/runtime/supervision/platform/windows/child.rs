use super::{
    GetExitCodeProcess, SupervisedChild, TerminateProcess, WaitForSingleObject, INFINITE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};

pub(in crate::infrastructure::runtime::supervision) struct PlatformChild {
    pub(super) process: std::os::windows::io::OwnedHandle,
    pub(super) stdout: Option<std::fs::File>,
    pub(super) stderr: Option<std::fs::File>,
}

impl SupervisedChild for PlatformChild {
    type Stdout = std::fs::File;
    type Stderr = std::fs::File;

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }
    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::io::AsRawHandle;
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => exit_status(self.process.as_raw_handle()),
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        use std::os::windows::io::AsRawHandle;
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(std::io::Error::last_os_error());
        }
        self.poll()?
            .ok_or_else(|| std::io::Error::other("process wait completed without an exit status"))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn exit_status(handle: *mut std::ffi::c_void) -> std::io::Result<Option<std::process::ExitStatus>> {
    use std::os::windows::process::ExitStatusExt;
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(handle, &mut exit_code) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(Some(std::process::ExitStatus::from_raw(exit_code)))
    }
}
