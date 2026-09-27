use super::super::super::seams::CommandSpec;
use super::{ProcessContainment, SupervisedChild};

mod child;
#[cfg(windows)]
mod ffi;
pub(in crate::infrastructure::runtime::supervision) use child::PlatformChild;
#[cfg(windows)]
pub(super) use ffi::WindowsJob as PlatformWindowsJob;
#[cfg(windows)]
use ffi::*;

/// Windows commands are created suspended, assigned to the configured
/// kill-on-close job, then resumed. This closes the spawn-to-job-assignment
/// race: no child instruction runs outside containment.
#[cfg(windows)]
pub(in crate::infrastructure::runtime::supervision) fn spawn_contained_command(
    spec: &CommandSpec,
) -> std::io::Result<(PlatformChild, ProcessContainment)> {
    let job = WindowsJob::create()?;
    let pipes = ChildPipes::create()?;
    let suspended = create_suspended_process(spec, &pipes)?;
    resume_in_job(&job, &suspended)?;
    Ok((
        pipes.into_child(suspended.process),
        ProcessContainment::from_job(job),
    ))
}

#[cfg(windows)]
struct ChildPipes {
    stdin_read: std::os::windows::io::OwnedHandle,
    stdout_read: std::os::windows::io::OwnedHandle,
    stdout_write: std::os::windows::io::OwnedHandle,
    stderr_read: std::os::windows::io::OwnedHandle,
    stderr_write: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ChildPipes {
    fn create() -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;

        let (stdin_read, stdin_write) = create_pipe()?;
        // Closing the parent writer gives the child a private EOF stdin even
        // in GUI processes without an inheritable console.
        drop(stdin_write);
        let (stdout_read, stdout_write) = create_pipe()?;
        let (stderr_read, stderr_write) = create_pipe()?;
        set_handle_inheritable(stdout_read.as_raw_handle(), false)?;
        set_handle_inheritable(stderr_read.as_raw_handle(), false)?;
        Ok(Self {
            stdin_read,
            stdout_read,
            stdout_write,
            stderr_read,
            stderr_write,
        })
    }

    fn inherited_handles(&self) -> [*mut std::ffi::c_void; 3] {
        use std::os::windows::io::AsRawHandle;

        [
            self.stdin_read.as_raw_handle(),
            self.stdout_write.as_raw_handle(),
            self.stderr_write.as_raw_handle(),
        ]
    }

    fn startup_info(&self, attribute_list: *mut std::ffi::c_void) -> StartupInfoExW {
        use std::os::windows::io::AsRawHandle;

        let mut startup = StartupInfoExW {
            startup_info: StartupInfoW {
                flags: STARTF_USESTDHANDLES,
                std_input: self.stdin_read.as_raw_handle(),
                std_output: self.stdout_write.as_raw_handle(),
                std_error: self.stderr_write.as_raw_handle(),
                ..StartupInfoW::default()
            },
            attribute_list,
        };
        startup.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
        startup
    }

    fn into_child(self, process: std::os::windows::io::OwnedHandle) -> PlatformChild {
        use std::fs::File;

        PlatformChild {
            process,
            stdout: Some(File::from(self.stdout_read)),
            stderr: Some(File::from(self.stderr_read)),
        }
    }
}

#[cfg(windows)]
struct SuspendedProcess {
    process: std::os::windows::io::OwnedHandle,
    thread: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
fn create_suspended_process(
    spec: &CommandSpec,
    pipes: &ChildPipes,
) -> std::io::Result<SuspendedProcess> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    let mut inherited_handles = pipes.inherited_handles();
    let process_attributes = ProcessAttributeList::for_handle_list(&mut inherited_handles)?;
    let mut startup = pipes.startup_info(process_attributes.as_ptr());
    let mut process_info = ProcessInformation::default();
    let mut command_line = windows_command_line(spec)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            (&mut startup as *mut StartupInfoExW).cast::<StartupInfoW>(),
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreateProcessW returned each owned handle exactly once.
    Ok(SuspendedProcess {
        process: unsafe { OwnedHandle::from_raw_handle(process_info.process) },
        thread: unsafe { OwnedHandle::from_raw_handle(process_info.thread) },
    })
}

#[cfg(windows)]
fn resume_in_job(job: &WindowsJob, suspended: &SuspendedProcess) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    if let Err(error) = job.assign(suspended.process.as_raw_handle()) {
        terminate_suspended_process(suspended.process.as_raw_handle());
        return Err(error);
    }
    if unsafe { ResumeThread(suspended.thread.as_raw_handle()) } == INVALID_DWORD {
        let error = std::io::Error::last_os_error();
        terminate_suspended_process(suspended.process.as_raw_handle());
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn create_pipe() -> std::io::Result<(
    std::os::windows::io::OwnedHandle,
    std::os::windows::io::OwnedHandle,
)> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    let attributes = SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor: std::ptr::null_mut(),
        inherit_handle: 1,
    };
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreatePipe returned one owned handle for each endpoint.
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    // SAFETY: CreatePipe returned one owned handle for each endpoint.
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    Ok((read, write))
}

#[cfg(windows)]
fn set_handle_inheritable(handle: *mut std::ffi::c_void, inheritable: bool) -> std::io::Result<()> {
    let flags = u32::from(inheritable) * HANDLE_FLAG_INHERIT;
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Owns the native attribute-list allocation for the entire CreateProcessW
/// call, deleting it on every success and failure path.
#[cfg(windows)]
struct ProcessAttributeList {
    attribute_list: *mut std::ffi::c_void,
    _storage: Vec<usize>,
}

#[cfg(windows)]
impl ProcessAttributeList {
    fn for_handle_list(handles: &mut [*mut std::ffi::c_void]) -> std::io::Result<Self> {
        let mut required_bytes = 0_usize;
        // The sizing call intentionally returns false and supplies the size.
        let _ = unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut required_bytes)
        };
        if required_bytes == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let word_size = std::mem::size_of::<usize>();
        let mut storage = vec![0_usize; required_bytes.div_ceil(word_size)];
        let attribute_list = storage.as_mut_ptr().cast::<std::ffi::c_void>();
        if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut required_bytes) }
            == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe {
            UpdateProcThreadAttribute(
                attribute_list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_mut_ptr().cast::<std::ffi::c_void>(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            // SAFETY: InitializeProcThreadAttributeList completed above.
            unsafe { DeleteProcThreadAttributeList(attribute_list) };
            return Err(error);
        }
        Ok(Self {
            attribute_list,
            _storage: storage,
        })
    }

    fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.attribute_list
    }
}

#[cfg(windows)]
impl Drop for ProcessAttributeList {
    fn drop(&mut self) {
        // SAFETY: this object is constructed only after successful
        // InitializeProcThreadAttributeList.
        unsafe { DeleteProcThreadAttributeList(self.attribute_list) };
    }
}

#[cfg(windows)]
fn terminate_suspended_process(process: *mut std::ffi::c_void) {
    let _ = unsafe { TerminateProcess(process, 1) };
    let _ = unsafe { WaitForSingleObject(process, INFINITE) };
}

#[cfg(windows)]
fn windows_command_line(spec: &CommandSpec) -> String {
    std::iter::once(&spec.program)
        .chain(spec.args.iter())
        .map(|argument| quote_windows_command_line_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn quote_windows_command_line_argument(argument: &str) -> String {
    if does_not_need_windows_quotes(argument) {
        return argument.into();
    }
    quote_windows_argument(argument)
}

fn does_not_need_windows_quotes(argument: &str) -> bool {
    !argument.is_empty()
        && !argument.contains(' ')
        && !argument.contains('\t')
        && !argument.contains('"')
}

fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}
