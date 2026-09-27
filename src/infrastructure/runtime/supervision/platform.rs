//! Operating-system process-tree containment primitives.

use std::io::Read;

#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, windows)))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub(super) use unix::*;
#[cfg(windows)]
pub(super) use windows::*;

pub(super) struct ProcessContainment {
    #[cfg(unix)]
    process_group: libc::pid_t,
    #[cfg(windows)]
    job: windows::PlatformWindowsJob,
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
    fn from_job(job: windows::PlatformWindowsJob) -> Self {
        Self { job }
    }

    pub(super) fn terminate<C: SupervisedChild>(&self, child: &mut C) {
        #[cfg(unix)]
        {
            // A negative PID targets the dedicated process group before the
            // direct child is reaped, so descendants cannot retain its pipes.
            let _ = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
        }
        #[cfg(windows)]
        {
            let _ = self.job.terminate();
        }
        let _ = child.kill();
    }
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
