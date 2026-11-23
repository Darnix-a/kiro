use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let counter = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("kiro-test-{}-{counter}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not make test directory: {error}"),
            }
        }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn report_runs_from_another_directory_and_writes_only_failed_checks() {
    let dir = TestDirectory::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let input = dir.0.join("checks with spaces.json");
    fs::copy(examples.join("checks.json"), &input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .current_dir(&dir.0)
        .arg(examples.join("report.kr"))
        .arg(input)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Wrote 1 failed check"));
    assert_eq!(
        fs::read_to_string(dir.0.join("failures.json")).unwrap(),
        r#"[{"failed":true,"name":"cache warmed"}]"#
    );
}

#[test]
fn uncaught_process_failure_reports_source_exit_status_and_stderr() {
    let dir = TestDirectory::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .current_dir(&dir.0)
        .arg(examples.join("report.kr"))
        .arg("missing checks.json")
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "report.kr:7:",
        "ProcessError",
        "exit status: 1",
        "\ncat:",
        "No such file or directory",
    ] {
        assert!(
            diagnostic.contains(expected),
            "missing {expected:?} in {diagnostic}"
        );
    }
    assert!(!dir.0.join("failures.json").exists());
}

#[test]
fn imports_hide_private_names_and_detect_cycles() {
    let dir = TestDirectory::new();
    let script = dir.0.join("main.kr");
    let helper = dir.0.join("helper.kr");
    let engine = kiro::Interpreter::cli();

    fs::write(
        &script,
        "import './helper.kr' as helper\nreturn helper.private",
    )
    .unwrap();
    fs::write(&helper, "private = 1\nexport public = 2").unwrap();
    assert_eq!(engine.run_file(&script).unwrap_err().kind, "KeyError");

    fs::write(&helper, "import './main.kr' as main").unwrap();
    let error = engine.run_file(&script).unwrap_err();
    assert_eq!(error.kind, "ImportError");
    assert!(error.message.contains("cyclic import"));
}
