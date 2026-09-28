use std::ffi::c_void;

pub(super) const HANDLE_FLAG_INHERIT: u32 = 1;
pub(super) const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
pub(super) const CREATE_SUSPENDED: u32 = 0x0000_0004;
pub(super) const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
pub(super) const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
pub(super) const WAIT_OBJECT_0: u32 = 0;
pub(super) const WAIT_TIMEOUT: u32 = 258;
pub(super) const INFINITE: u32 = u32::MAX;
pub(super) const INVALID_DWORD: u32 = u32::MAX;

const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

pub(in crate::infrastructure::runtime::supervision::platform) struct WindowsJob {
    handle: *mut c_void,
}

impl WindowsJob {
    pub(super) fn create() -> std::io::Result<Self> {
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

    pub(super) fn assign(&self, process: *mut c_void) -> std::io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(in crate::infrastructure::runtime::supervision::platform) fn terminate(
        &self,
    ) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for WindowsJob {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE is the backstop for every error and unwind path.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

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

impl JobObjectExtendedLimitInformation {
    fn kill_on_close() -> Self {
        let mut limits = Self::default();
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits
    }
}

#[repr(C)]
pub(super) struct SecurityAttributes {
    pub(super) length: u32,
    pub(super) security_descriptor: *mut c_void,
    pub(super) inherit_handle: i32,
}

#[repr(C)]
pub(super) struct StartupInfoW {
    pub(super) cb: u32,
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
    pub(super) flags: u32,
    show_window: u16,
    reserved2_count: u16,
    reserved2: *mut u8,
    pub(super) std_input: *mut c_void,
    pub(super) std_output: *mut c_void,
    pub(super) std_error: *mut c_void,
}

/// Exact `STARTUPINFOEXW` layout: a `STARTUPINFOW` prefix followed by the
/// optional attribute-list pointer used to whitelist inherited handles.
#[repr(C)]
pub(super) struct StartupInfoExW {
    pub(super) startup_info: StartupInfoW,
    pub(super) attribute_list: *mut c_void,
}

impl Default for StartupInfoExW {
    fn default() -> Self {
        Self {
            startup_info: StartupInfoW::default(),
            attribute_list: std::ptr::null_mut(),
        }
    }
}

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

impl StartupInfoW {
    pub(super) fn with_standard_handles(
        std_input: *mut c_void,
        std_output: *mut c_void,
        std_error: *mut c_void,
    ) -> Self {
        let mut startup = Self::default();
        startup.flags = STARTF_USESTDHANDLES;
        startup.std_input = std_input;
        startup.std_output = std_output;
        startup.std_error = std_error;
        startup
    }
}

#[repr(C)]
#[derive(Default)]
pub(super) struct ProcessInformation {
    pub(super) process: *mut c_void,
    pub(super) thread: *mut c_void,
    process_id: u32,
    thread_id: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub(super) fn CreateJobObjectW(job_attributes: *const c_void, name: *const u16) -> *mut c_void;
    pub(super) fn SetInformationJobObject(
        job: *mut c_void,
        information_class: i32,
        information: *const c_void,
        information_length: u32,
    ) -> i32;
    pub(super) fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    pub(super) fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
    pub(super) fn CloseHandle(handle: *mut c_void) -> i32;
    pub(super) fn CreatePipe(
        read_pipe: *mut *mut c_void,
        write_pipe: *mut *mut c_void,
        attributes: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    pub(super) fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub(super) fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    pub(super) fn ResumeThread(thread: *mut c_void) -> u32;
    pub(super) fn TerminateProcess(process: *mut c_void, exit_code: u32) -> i32;
    pub(super) fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    pub(super) fn GetExitCodeProcess(process: *mut c_void, exit_code: *mut u32) -> i32;
    pub(super) fn InitializeProcThreadAttributeList(
        attribute_list: *mut c_void,
        attribute_count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    pub(super) fn UpdateProcThreadAttribute(
        attribute_list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous_value: *mut c_void,
        return_size: *mut usize,
    ) -> i32;
    pub(super) fn DeleteProcThreadAttributeList(attribute_list: *mut c_void);
}
