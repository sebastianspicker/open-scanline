## What changed

<!-- Describe the change and its user-visible effect. If it only affects a
     specific platform or backend, say which one. -->

## How to test

<!-- Give a reviewer the command, GUI path, or fixture to reproduce it. -->

## Checklist

- [ ] `sh scripts/check_architecture.sh`
- [ ] `bash scripts/check_code_quality.sh`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo clippy --all-targets --no-default-features --locked -- -D warnings`
- [ ] `cargo test --all-features --locked`
- [ ] `cargo test --no-default-features --locked`

## Not run

<!-- List any platform-specific, hardware, or external-tool checks you could not
     run, and what would be needed to run them. -->

<!-- Reminder: never include credentials, scanner addresses, captured documents,
     or other sensitive data in a pull request. -->
