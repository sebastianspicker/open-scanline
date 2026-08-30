//! Bounded concurrent command-pipe draining.

use crate::error::{Result, ScanError};
use std::io::Read;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

pub(super) const COMMAND_STREAM_DRAIN_GRACE: Duration = Duration::from_millis(250);

pub(super) struct DrainedCommandStream {
    pub(super) bytes: Vec<u8>,
    pub(super) overflowed: bool,
}

pub(super) struct CommandStreamReader {
    result: Receiver<std::io::Result<DrainedCommandStream>>,
    thread: JoinHandle<()>,
}

pub(super) fn drain_command_stream<R: Read + Send + 'static>(
    mut stream: R,
    capture_limit: usize,
) -> CommandStreamReader {
    let (sender, result) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let drained = (|| {
            let mut bytes = Vec::new();
            let mut overflowed = false;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    return Ok(DrainedCommandStream { bytes, overflowed });
                }
                let remaining = capture_limit.saturating_sub(bytes.len());
                let captured = remaining.min(count);
                bytes.extend_from_slice(&buffer[..captured]);
                overflowed |= captured < count;
            }
        })();
        let _ = sender.send(drained);
    });
    CommandStreamReader { result, thread }
}

pub(super) fn join_command_stream(reader: CommandStreamReader) -> Result<DrainedCommandStream> {
    let result = reader
        .result
        .recv_timeout(COMMAND_STREAM_DRAIN_GRACE)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => ScanError::Unsupported(
                "command output streams remained open after the command exited".into(),
            ),
            mpsc::RecvTimeoutError::Disconnected => {
                ScanError::Unsupported("command output reader disconnected".into())
            }
        })?;
    reader
        .thread
        .join()
        .map_err(|_| ScanError::Unsupported("command output reader panicked".into()))?;
    result.map_err(|error| ScanError::Unsupported(format!("command output read failed: {error}")))
}
