//! Operating-system process-tree containment primitives.

use super::super::super::seams::CommandSpec;
use super::{ProcessContainment, SupervisedChild};

#[cfg(not(windows))]
use std::process::{Command, Stdio};

#[cfg(not(windows))]
pub(in crate::infrastructure::runtime::supervision) fn spawn_contained_command(
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
pub(super) type PlatformChild = std::process::Child;

#[cfg(not(any(unix, windows)))]
pub(super) type PlatformChild = std::process::Child;

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
