//! Enforces the one-way layering documented in `docs/architecture.md`:
//! `inbound` -> `workflows` -> `infrastructure` -> `domain`.

use std::fs;
use std::path::{Path, PathBuf};

/// Layer directory and the module paths that layer must never reference.
const RULES: &[(&str, &[&str])] = &[
    (
        "domain",
        &[
            "crate::infrastructure",
            "crate::inbound",
            "crate::workflows",
        ],
    ),
    ("infrastructure", &["crate::inbound", "crate::workflows"]),
    ("workflows", &["crate::inbound"]),
];

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn layers_only_depend_downwards() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for (layer, forbidden) in RULES {
        let mut files = Vec::new();
        collect_sources(&root.join("src").join(layer), &mut files);
        files.sort();
        for file in files {
            let text = fs::read_to_string(&file).expect("readable source file");
            for (index, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if let Some(target) = forbidden.iter().find(|target| line.contains(**target)) {
                    violations.push(format!(
                        "{}:{}: src/{layer} must not reference {target}",
                        file.strip_prefix(root).unwrap_or(&file).display(),
                        index + 1
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "layering violations:\n{}",
        violations.join("\n")
    );
}
