//! Runs the shell-level installer tests (tests/install_hook_test.sh) under
//! `cargo test` so the Claude Code hook-registration logic in install.sh is
//! covered by CI. Requires `bash` and `python3`, which install.sh itself needs.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn install_hook_registration() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("install_hook_test.sh");

    let output = Command::new("bash")
        .arg(&script)
        .output()
        .expect("failed to run install_hook_test.sh (is bash on PATH?)");

    // Surface the script's own ok/FAIL lines on failure.
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    assert!(
        output.status.success(),
        "install_hook_test.sh failed (see output above)"
    );
}
