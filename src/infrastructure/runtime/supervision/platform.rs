//! Operating-system process-tree containment primitives.

use super::super::seams::CommandSpec;
use std::io::Read;

#[cfg(not(windows))]
use std::process::{Command, Stdio};

#[cfg(not(windows))]
pub(super) fn spawn_contained_command(
    spec: &CommandSpec,
) -> std::io::Result<(PlatformChild, ProcessContainment)> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_containment(&mut command);
    let mut child = command.spawn()?;
    let containment = match ProcessContainment::for_child(&child) {
        Ok(containment) => containment,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    Ok((child, containment))
}

#[cfg(unix)]
type PlatformChild = std::process::Child;

#[cfg(not(any(unix, windows)))]
type PlatformChild = std::process::Child;

/// Unix children start in their own process group. Killing that group reaches
/// ordinary descendants before the direct child is reaped. A descendant that
/// deliberately calls `setsid` or otherwise changes process groups escapes
/// this containment boundary by design.
#[cfg(unix)]
fn configure_process_containment(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Keep the command tree out of the parent's terminal process group. The
    // standard API can use posix_spawn on supported platforms; a custom
    // pre_exec hook forces fork and can run GUI-framework atfork handlers that
    // restore SIGINT's default disposition in an all-features CLI process.
    command.process_group(0);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_containment(_: &mut Command) {}

pub(super) struct ProcessContainment {
    #[cfg(unix)]
    process_group: libc::pid_t,
    #[cfg(windows)]
    job: WindowsJob,
}

impl ProcessContainment {
    #[cfg(not(windows))]
    fn for_child(child: &PlatformChild) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                process_group: child.id() as libc::pid_t,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            Ok(Self {})
        }
    }

    #[cfg(windows)]
    fn from_job(job: WindowsJob) -> Self {
        Self { job }
    }

    pub(super) fn terminate<C: SupervisedChild>(&self, child: &mut C) {
        #[cfg(unix)]
        {
            // A negative PID targets the dedicated process group. Do this
            // before waiting so descendants cannot retain command pipes.
            let _ = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
        }
        #[cfg(windows)]
        {
            let _ = self.job.terminate();
        }
        let _ = child.kill();
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl WindowsJob {
    fn create() -> std::io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Self { handle };
        let mut limits = JobObjectExtendedLimitInformation::kill_on_close();
        let configured = unsafe {
            SetInformationJobObject(
                job.handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                (&mut limits as *mut JobObjectExtendedLimitInformation).cast(),
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };
        if configured == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, process: *mut std::ffi::c_void) -> std::io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn terminate(&self) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE is the backstop for every error and unwind path.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
#[cfg(windows)]
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[cfg(windows)]
impl JobObjectExtendedLimitInformation {
    fn kill_on_close() -> Self {
        let mut limits = Self::default();
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(
        job_attributes: *const std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn SetInformationJobObject(
        job: *mut std::ffi::c_void,
        information_class: u32,
        information: *const std::ffi::c_void,
        information_length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn CreatePipe(
        read_pipe: *mut *mut std::ffi::c_void,
        write_pipe: *mut *mut std::ffi::c_void,
        attributes: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const std::ffi::c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    fn ResumeThread(thread: *mut std::ffi::c_void) -> u32;
    fn TerminateProcess(process: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: *mut std::ffi::c_void, exit_code: *mut u32) -> i32;
    fn InitializeProcThreadAttributeList(
        attribute_list: *mut std::ffi::c_void,
        attribute_count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        attribute_list: *mut std::ffi::c_void,
        flags: u32,
        attribute: usize,
        value: *mut std::ffi::c_void,
        size: usize,
        previous_value: *mut std::ffi::c_void,
        return_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(attribute_list: *mut std::ffi::c_void);
}

#[cfg(windows)]
const HANDLE_FLAG_INHERIT: u32 = 1;
#[cfg(windows)]
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;
#[cfg(windows)]
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
#[cfg(windows)]
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
#[cfg(windows)]
const WAIT_OBJECT_0: u32 = 0;
#[cfg(windows)]
const WAIT_TIMEOUT: u32 = 258;
#[cfg(windows)]
const INFINITE: u32 = u32::MAX;
#[cfg(windows)]
const INVALID_DWORD: u32 = u32::MAX;

#[cfg(windows)]
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut std::ffi::c_void,
    inherit_handle: i32,
}

#[cfg(windows)]
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_count: u16,
    reserved2: *mut u8,
    std_input: *mut std::ffi::c_void,
    std_output: *mut std::ffi::c_void,
    std_error: *mut std::ffi::c_void,
}

/// Exact `STARTUPINFOEXW` layout: a `STARTUPINFOW` prefix followed by the
/// optional attribute-list pointer used to whitelist inherited handles.
#[cfg(windows)]
#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    attribute_list: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl Default for StartupInfoExW {
    fn default() -> Self {
        Self {
            startup_info: StartupInfoW::default(),
            attribute_list: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl Default for StartupInfoW {
    fn default() -> Self {
        Self {
            cb: std::mem::size_of::<Self>() as u32,
            reserved: std::ptr::null_mut(),
            desktop: std::ptr::null_mut(),
            title: std::ptr::null_mut(),
            x: 0,
            y: 0,
            x_size: 0,
            y_size: 0,
            x_count_chars: 0,
            y_count_chars: 0,
            fill_attribute: 0,
            flags: 0,
            show_window: 0,
            reserved2_count: 0,
            reserved2: std::ptr::null_mut(),
            std_input: std::ptr::null_mut(),
            std_output: std::ptr::null_mut(),
            std_error: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ProcessInformation {
    process: *mut std::ffi::c_void,
    thread: *mut std::ffi::c_void,
    process_id: u32,
    thread_id: u32,
}

/// Windows commands are created suspended, assigned to the configured
/// kill-on-close job, then resumed. This closes the spawn-to-job-assignment
/// race: no child instruction runs outside containment.
#[cfg(windows)]
pub(super) fn spawn_contained_command(
    spec: &CommandSpec,
) -> std::io::Result<(PlatformChild, ProcessContainment)> {
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    let job = WindowsJob::create()?;
    let (stdin_read, stdin_write) = create_pipe()?;
    // Closing the parent writer before creation gives every command a valid,
    // private EOF stdin even in GUI processes without an inheritable console.
    drop(stdin_write);
    let (stdout_read, stdout_write) = create_pipe()?;
    let (stderr_read, stderr_write) = create_pipe()?;
    set_handle_inheritable(stdout_read.as_raw_handle(), false)?;
    set_handle_inheritable(stderr_read.as_raw_handle(), false)?;
    let mut inherited_handles = [
        stdin_read.as_raw_handle(),
        stdout_write.as_raw_handle(),
        stderr_write.as_raw_handle(),
    ];
    let process_attributes = ProcessAttributeList::for_handle_list(&mut inherited_handles)?;
    let mut startup = StartupInfoExW {
        startup_info: StartupInfoW {
            flags: STARTF_USESTDHANDLES,
            std_input: stdin_read.as_raw_handle(),
            std_output: stdout_write.as_raw_handle(),
            std_error: stderr_write.as_raw_handle(),
            ..StartupInfoW::default()
        },
        attribute_list: process_attributes.as_ptr(),
    };
    // EXTENDED_STARTUPINFO_PRESENT requires cb to describe the complete
    // STARTUPINFOEXW structure, not only its STARTUPINFOW prefix.
    startup.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
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

    // SAFETY: CreateProcessW returned these owned handles exactly once.
    let process = unsafe { OwnedHandle::from_raw_handle(process_info.process) };
    // SAFETY: CreateProcessW returned these owned handles exactly once.
    let thread = unsafe { OwnedHandle::from_raw_handle(process_info.thread) };
    if let Err(error) = job.assign(process.as_raw_handle()) {
        terminate_suspended_process(process.as_raw_handle());
        return Err(error);
    }
    if unsafe { ResumeThread(thread.as_raw_handle()) } == INVALID_DWORD {
        let error = std::io::Error::last_os_error();
        terminate_suspended_process(process.as_raw_handle());
        return Err(error);
    }
    drop(thread);
    drop(stdout_write);
    drop(stderr_write);
    Ok((
        PlatformChild {
            process,
            stdout: Some(File::from(stdout_read)),
            stderr: Some(File::from(stderr_read)),
        },
        ProcessContainment::from_job(job),
    ))
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
    if !argument.is_empty()
        && !argument.contains(' ')
        && !argument.contains('\t')
        && !argument.contains('"')
    {
        return argument.into();
    }
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

#[cfg(windows)]
struct PlatformChild {
    process: std::os::windows::io::OwnedHandle,
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
}

pub(super) trait SupervisedChild {
    type Stdout: Read + Send + 'static;
    type Stderr: Read + Send + 'static;

    fn take_stdout(&mut self) -> Option<Self::Stdout>;
    fn take_stderr(&mut self) -> Option<Self::Stderr>;
    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn kill(&mut self) -> std::io::Result<()>;
}

impl SupervisedChild for std::process::Child {
    type Stdout = std::process::ChildStdout;
    type Stderr = std::process::ChildStderr;

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        std::process::Child::wait(self)
    }

    fn kill(&mut self) -> std::io::Result<()> {
        std::process::Child::kill(self)
    }
}

#[cfg(windows)]
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
        use std::os::windows::process::ExitStatusExt;

        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut exit_code) } == 0
                {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(Some(std::process::ExitStatus::from_raw(exit_code)))
                }
            }
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        use std::os::windows::io::AsRawHandle;

        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(std::io::Error::last_os_error());
        }
        self.poll()?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "process wait completed without an exit status",
            )
        })
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
