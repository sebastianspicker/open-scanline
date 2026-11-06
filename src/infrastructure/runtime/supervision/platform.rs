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
            let group = self.process_group;
            // A negative PID targets the dedicated process group.
            escalate_termination(child, UNIX_TERMINATE_GRACE, |signal| {
                let _ = unsafe { libc::kill(-group, signal) };
            });
        }
        #[cfg(windows)]
        {
            let _ = self.job.terminate();
            let _ = child.kill();
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child.kill();
        }
    }
}

/// `scanimage` handles SIGTERM by calling `sane_cancel`, which releases the
/// device; a slow feeder cancel takes about 1 s.
#[cfg(unix)]
const UNIX_TERMINATE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// SIGTERM the group, give the child `grace` to exit, then SIGKILL the group.
///
/// The group is always SIGKILLed at the end, even after the direct child has
/// exited: descendants that still hold its pipes must not survive. A group id
/// stays reserved while any member lives, so this only risks a wrong target
/// when the group is already empty and its number was reused as a new group
/// leader, which is very rare.
#[cfg(unix)]
fn escalate_termination<C: SupervisedChild>(
    child: &mut C,
    grace: std::time::Duration,
    mut signal_group: impl FnMut(libc::c_int),
) {
    let exited = |child: &mut C| matches!(child.poll(), Ok(Some(_)));
    // A child that already finished needs no graceful cancel; only clean up.
    if !exited(child) {
        signal_group(libc::SIGTERM);
        let deadline = std::time::Instant::now() + grace;
        while std::time::Instant::now() < deadline && !exited(child) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    signal_group(libc::SIGKILL);
    let _ = child.kill();
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Default)]
    struct FakeChild {
        exits_after_polls: Option<u32>,
        polls: u32,
        killed: bool,
    }

    impl SupervisedChild for FakeChild {
        type Stdout = std::io::Empty;
        type Stderr = std::io::Empty;

        fn take_stdout(&mut self) -> Option<Self::Stdout> {
            None
        }
        fn take_stderr(&mut self) -> Option<Self::Stderr> {
            None
        }
        fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
            self.polls += 1;
            let exited = self
                .exits_after_polls
                .is_some_and(|limit| self.polls > limit);
            Ok(exited.then(|| std::os::unix::process::ExitStatusExt::from_raw(0)))
        }
        fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
            unreachable!()
        }
        fn kill(&mut self) -> std::io::Result<()> {
            self.killed = true;
            Ok(())
        }
    }

    fn signals(child: &mut FakeChild) -> Vec<libc::c_int> {
        let mut sent = Vec::new();
        escalate_termination(child, Duration::from_millis(50), |signal| sent.push(signal));
        sent
    }

    #[test]
    fn a_stuck_child_gets_term_then_kill() {
        let mut child = FakeChild::default();
        assert_eq!(signals(&mut child), [libc::SIGTERM, libc::SIGKILL]);
        assert!(child.killed);
    }

    #[test]
    fn a_child_that_exits_in_the_grace_stops_waiting_early() {
        let mut child = FakeChild {
            exits_after_polls: Some(3),
            ..FakeChild::default()
        };
        // The final group SIGKILL only cleans up descendants holding pipes.
        assert_eq!(signals(&mut child), [libc::SIGTERM, libc::SIGKILL]);
        assert_eq!(child.polls, 4);
    }

    #[test]
    fn a_finished_child_gets_no_graceful_cancel() {
        let mut child = FakeChild {
            exits_after_polls: Some(0),
            ..FakeChild::default()
        };
        assert_eq!(signals(&mut child), [libc::SIGKILL]);
    }
}
