use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

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

#[test]
fn a_standalone_command_inherits_terminal_streams_and_checks_status() {
    let dir = TestDirectory::new();
    let path = dir.0.join("stream.kr");
    let helper = dir.0.join("stream helper.sh");
    fs::write(&path, "script = args[0]\n$ \"sh {script}\"").unwrap();
    fs::write(
        &helper,
        "printf '\\033[32mready\\033[0m\\n'\nprintf 'waiting\\n' >&2\nIFS= read -r release || exit 11\nexit 7",
    )
    .unwrap();
    let mut kr = Command::new(env!("CARGO_BIN_EXE_kr"))
        .arg(path)
        .arg(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    let mut stdout = BufReader::new(kr.stdout.take().unwrap());
    let mut stderr = BufReader::new(kr.stderr.take().unwrap());
    let read_stdout = std::thread::spawn(move || {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let _ = stdout_tx.send(line);
    });
    let read_stderr = std::thread::spawn(move || {
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        let _ = stderr_tx.send(line);
        let mut rest = String::new();
        stderr.read_to_string(&mut rest).unwrap();
        rest
    });
    let stdout_first = stdout_rx.recv_timeout(Duration::from_secs(3));
    let stderr_first = stderr_rx.recv_timeout(Duration::from_secs(3));
    let mut stdin = kr.stdin.take().unwrap();
    let _ = stdin.write_all(b"release\n");
    drop(stdin);
    let status = kr.wait().unwrap();
    read_stdout.join().unwrap();
    let error = read_stderr.join().unwrap();

    assert_eq!(stdout_first.unwrap(), "\u{1b}[32mready\u{1b}[0m\n");
    assert_eq!(stderr_first.unwrap(), "waiting\n");
    assert_eq!(status.code(), Some(1));
    assert!(error.contains("stream.kr:2:1: ProcessError"), "{error}");
    assert!(error.contains("exit status: 7"), "{error}");
}

#[test]
fn inline_scripts_accept_arguments_and_exit_with_a_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .args(["-e", "print(args[0])\ntry:\n    exit(27)\ncatch err:\n    print(err.kind)\nprint(\"unreachable\")", "hello world"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(27));
    assert_eq!(output.stdout, b"hello world\n");
    assert!(output.stderr.is_empty());

    let status = Command::new(env!("CARGO_BIN_EXE_kr"))
        .args(["-e", "exit()"])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .arg("-e")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("-e requires"));
}

#[test]
fn input_prints_its_prompt_and_reads_a_line_then_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kr"))
        .args([
            "-e",
            "name = input(\"Name? \")\nprint(\"Hi {name}\")\nprint(input() == null)",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"Kiro\r\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"Name? Hi Kiro\ntrue\n");
}

#[test]
fn filesystem_operations_work_on_files_and_trees() {
    let dir = TestDirectory::new();
    let script = r#"
fs.mkdir("logs/nested")
fs.mkdir("logs/nested")
assertions = [fs.exists("logs"), fs.is_dir("logs"), not fs.is_file("logs")]
fs.append_text("logs/b.log", "one\r\n")
fs.append_text("logs/b.log", "two\n")
fs.write_text("logs/a.log", "other")
fs.write_text("logs/nested/hidden", "nested")
assertions.append(fs.is_file("logs/b.log"))
assertions.append(fs.read_lines("logs/b.log") == ["one", "two"])
assertions.append(fs.list_dir("logs") == ["a.log", "b.log", "nested"])
assertions.append(fs.glob("./logs/*.log") == ["logs/a.log", "logs/b.log"])
fs.copy("logs", "backup")
fs.copy("backup/a.log", "extra.log")
fs.move("backup/b.log", "moved.log")
fs.move("backup/nested", "archive")
assertions.append(fs.read_text("archive/hidden") == "nested")
assertions.append(fs.read_lines("moved.log") == ["one", "two"])
assertions.append(not fs.exists("backup/b.log"))
try:
    fs.copy("logs", "logs/nested/recursive")
catch err:
    assertions.append(err.kind == "IoError")
fs.remove("extra.log")
fs.remove("logs")
assertions.append(not fs.exists("extra.log") and not fs.is_dir("logs"))
print(len(assertions), all(assertions, value => value))
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .current_dir(&dir.0)
        .args(["-e", script])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"12 true\n");
}

#[test]
fn environment_updates_reach_both_command_forms() {
    let output = Command::new(env!("CARGO_BIN_EXE_kr"))
        .args([
            "-e",
            r#"
env.set("KIRO_TEST_CHILD_ENV", "Kiro value")
print(env.get("KIRO_TEST_CHILD_ENV"), env.all()["KIRO_TEST_CHILD_ENV"])
print(($ "printenv KIRO_TEST_CHILD_ENV").out)
print(process.run("printenv", ["KIRO_TEST_CHILD_ENV"]).stdout.trim())
"#,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        b"Kiro value Kiro value\nKiro value\nKiro value\n"
    );
}

#[test]
fn http_serve_routes_requests_and_serializes_json_responses() {
    use std::net::{TcpListener, TcpStream};
    use std::time::Instant;

    struct TestServer(std::process::Child);
    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    let source = r#"
fn handler(req):
    match req.path:
        case "/echo" if req.method == "POST":
            return {
                status: 201,
                headers: {"x-kiro": "yes"},
                body: {greeting: req.query.greeting, tag: req.query.tag, input: json.parse(req.body), test: req.headers["x-test"][0]},
            }
        case "/stop":
            exit()
        case _:
            return {status: 404, body: "missing"}
"#;
    let script = format!("{source}\nhttp.serve({}, handler)", addr.port());
    let mut server = TestServer(
        Command::new(env!("CARGO_BIN_EXE_kr"))
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );

    let exchange = |method: &str, path: &str, body: &str| {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut socket = loop {
            match TcpStream::connect(addr) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() >= deadline => {
                    panic!("server failed to listen: {error}")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(socket, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nX-Test: active\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    };

    let response = exchange(
        "POST",
        "/echo?greeting=hello+world&tag=x%2By",
        r#"{"ok":true}"#,
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    assert!(response.to_ascii_lowercase().contains("x-kiro: yes"));
    let body = response.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        serde_json::json!({"greeting":"hello world","tag":"x+y","input":{"ok":true},"test":"active"})
    );
    assert!(exchange("GET", "/other", "").starts_with("HTTP/1.1 404"));
    assert!(exchange("GET", "/stop", "").starts_with("HTTP/1.1 204"));
    assert!(server.0.wait().unwrap().success());
}

#[test]
fn no_arguments_accepts_piped_source_without_prompts() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kr"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"base = 40\nfn add(n):\n    return base + n\nadd(2)\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
}
