use super::*;
use crate::infrastructure::runtime::TemporaryOutput;

fn quota() -> ArtifactQuota {
    ArtifactQuota {
        max_files: 1,
        max_bytes: 4,
    }
}

#[test]
fn every_poll_observes_growth_and_replacement() {
    let output = TemporaryOutput::new("artifact-watch-test", "bin").unwrap();
    fs::write(output.path(), [1; 4]).unwrap();
    validate_artifact_quota(output.directory(), quota()).unwrap();
    fs::write(output.path(), [1; 5]).unwrap();
    assert!(validate_artifact_quota(output.directory(), quota()).is_err());
    fs::remove_file(output.path()).unwrap();
    fs::create_dir(output.path()).unwrap();
    assert!(validate_artifact_quota(output.directory(), quota()).is_err());
}

#[test]
fn fresh_metadata_even_when_entry_was_enumerated_before_write() {
    let output = TemporaryOutput::new("artifact-fresh-test", "bin").unwrap();
    fs::write(output.path(), [1]).unwrap();
    let entry = fs::read_dir(output.directory())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    fs::write(output.path(), [2; 5]).unwrap();
    assert_eq!(fresh_entry_metadata(&entry).unwrap().len(), 5);
}

#[test]
fn file_count_and_final_contents_are_checked() {
    let output = TemporaryOutput::new("artifact-count-test", "bin").unwrap();
    fs::write(output.path(), []).unwrap();
    validate_artifact_quota(output.directory(), quota()).unwrap();
    fs::write(output.directory().join("another.bin"), []).unwrap();
    assert!(validate_artifact_quota(output.directory(), quota()).is_err());
}

#[cfg(unix)]
#[test]
fn replacement_symlink_is_never_followed() {
    let output = TemporaryOutput::new("artifact-link-test", "bin").unwrap();
    fs::write(output.path(), [1]).unwrap();
    let entry = fs::read_dir(output.directory())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    fs::remove_file(output.path()).unwrap();
    std::os::unix::fs::symlink("/dev/null", output.path()).unwrap();
    assert!(fresh_entry_metadata(&entry)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(validate_artifact_quota(output.directory(), quota()).is_err());
}
