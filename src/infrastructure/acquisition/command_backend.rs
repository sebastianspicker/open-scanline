//! Shared command-line-tool [`CommandRunner`] used by the SANE and WIA
//! adapters, which differ only in the program label and cancellation message
//! they attach to failures. Each backend exposes its own public unit-struct
//! `SystemCommandRunner` (declared with [`system_command_runner!`]) that
//! delegates here.

use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::ScanPagesResult;
use crate::infrastructure::runtime::{
    run_contained_command, run_contained_command_with_artifact_quota, ArtifactWatch, CommandOutput,
    CommandRunner, CommandSpec,
};
use crate::operation::CancellationToken;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Runs a command-line acquisition tool as a real OS process.
///
/// `program_label` names the tool in "not found"/failure messages;
/// `cancelled_message` is the [`crate::error::ScanError::Cancelled`] text used
/// when the operation-wide token or the per-call flag is already set.
pub(crate) struct ToolCommandRunner {
    program_label: &'static str,
    cancelled_message: &'static str,
}

impl ToolCommandRunner {
    pub(crate) const fn new(program_label: &'static str, cancelled_message: &'static str) -> Self {
        Self {
            program_label,
            cancelled_message,
        }
    }
}

impl CommandRunner for ToolCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
        artifacts: Option<ArtifactWatch<'_>>,
    ) -> Result<CommandOutput> {
        match artifacts {
            None => run_contained_command(
                spec,
                timeout,
                cancelled,
                cancellation,
                self.program_label,
                self.cancelled_message,
            ),
            Some(watch) => run_contained_command_with_artifact_quota(
                spec,
                timeout,
                cancelled,
                cancellation,
                self.program_label,
                self.cancelled_message,
                watch,
            ),
        }
    }
}

/// Declares a backend's public `SystemCommandRunner` unit struct, which runs
/// its tool through [`ToolCommandRunner`] with the given label and message.
macro_rules! system_command_runner {
    ($(#[$meta:meta])* $program_label:literal, $cancelled_message:literal) => {
        $(#[$meta])*
        pub struct SystemCommandRunner;

        impl SystemCommandRunner {
            const TOOL: $crate::infrastructure::acquisition::command_backend::ToolCommandRunner =
                $crate::infrastructure::acquisition::command_backend::ToolCommandRunner::new(
                    $program_label,
                    $cancelled_message,
                );
        }

        impl $crate::infrastructure::runtime::CommandRunner for SystemCommandRunner {
            fn run(
                &self,
                spec: &$crate::infrastructure::runtime::CommandSpec,
                timeout: ::std::time::Duration,
                cancelled: &::std::sync::Mutex<bool>,
                cancellation: Option<&$crate::operation::CancellationToken>,
                artifacts: Option<$crate::infrastructure::runtime::ArtifactWatch<'_>>,
            ) -> $crate::error::Result<$crate::infrastructure::runtime::CommandOutput> {
                Self::TOOL.run(spec, timeout, cancelled, cancellation, artifacts)
            }
        }
    };
}
pub(crate) use system_command_runner;

/// `Ok` when a command-backed batch emitted no more than `maximum` pages;
/// otherwise a descriptive error naming `program_label` (e.g. `"scanimage"`
/// or `"WIA"`).
pub(crate) fn validate_batch_page_count(
    actual: usize,
    maximum: u32,
    program_label: &str,
) -> Result<()> {
    if actual <= maximum as usize {
        return Ok(());
    }
    Err(ScanError::Other(format!(
        "{program_label} emitted {actual} pages beyond the requested limit {maximum}"
    )))
}

/// [`ScanPagesResult::limit_reached`] when every requested page was emitted,
/// otherwise [`ScanPagesResult::feeder_exhausted`].
pub(crate) fn command_backed_pages_result(emitted: u32, maximum: u32) -> ScanPagesResult {
    if emitted == maximum {
        ScanPagesResult::limit_reached(emitted)
    } else {
        ScanPagesResult::feeder_exhausted(emitted)
    }
}

/// Lower-case driver phrases that mean the document feeder ran out of paper.
pub(crate) const FEEDER_EXHAUSTED_PHRASES: &[&str] = &[
    "out of documents",
    "no documents",
    "document feeder empty",
    "paper empty",
    "no paper",
];

/// Whether `text` (any case) contains one of [`FEEDER_EXHAUSTED_PHRASES`].
pub(crate) fn mentions_feeder_exhausted(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    FEEDER_EXHAUSTED_PHRASES
        .iter()
        .any(|phrase| text.contains(phrase))
}

/// The complete `page_<digits>.<extension>` files in `directory`, in page
/// number order. In-progress names such as `page_000001.png.part` and every
/// other file are ignored.
pub(crate) fn numbered_page_outputs(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut pages = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        // `file_type` does not follow symlinks, so a planted link is never decoded.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter_map(|path| Some((page_number(&path, extension)?, path)))
        .collect::<Vec<_>>();
    pages.sort_by_key(|(number, _)| *number);
    Ok(pages.into_iter().map(|(_, path)| path).collect())
}

fn page_number(path: &Path, extension: &str) -> Option<u32> {
    let digits = path
        .file_name()?
        .to_str()?
        .strip_prefix("page_")?
        .strip_suffix(extension)?
        .strip_suffix('.')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::numbered_page_outputs;
    use crate::infrastructure::runtime::TemporaryOutput;

    #[test]
    fn numbered_outputs_sort_by_page_number_and_skip_other_names() {
        let output = TemporaryOutput::new("numbered-test", "png").unwrap();
        for name in [
            "page_000010.png",
            "page_000009.png",
            "page_000011.png.part",
            "page_.png",
            "page_00x001.png",
            "page_000001.jpg",
            "notes.png",
        ] {
            std::fs::write(output.directory().join(name), b"x").unwrap();
        }
        let names = numbered_page_outputs(output.directory(), "png")
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["page_000009.png", "page_000010.png"]);
    }
}
