use kiro::{Interpreter, Value};

#[test]
fn functions_are_local_and_blocks_share_their_surrounding_scope() {
    let result = Interpreter::new()
        .run_source(
            r#"
total = 0
fn sum(values):
    total = 0
    for value in values:
        if value > 0:
            total = total + value
    return total

if true:
    result = sum([
        2,
        -1,
        3,
    ])
return [total, result]
"#,
            "scope.kr",
        )
        .unwrap();
    assert_eq!(result, Value::list(vec![0.into(), 5.into()]));
}

#[test]
fn functions_use_lexical_scope_and_a_failed_assignment_does_not_fall_back() {
    let engine = Interpreter::new();
    let result = engine.run_source(
        "fn make(x):\n    fn add(y):\n        return x + y\n    return add\nadd = make(5)\nreturn add(4)",
        "closure.kr",
    ).unwrap();
    assert_eq!(result, Value::Int(9));

    let error = engine
        .run_source(
            "x = 3\nfn read():\n    if false:\n        x = 4\n    return x\nreturn read()",
            "names.kr",
        )
        .unwrap_err();
    assert_eq!(error.kind, "NameError");
    assert_eq!(error.location.unwrap().line, 5);
}

#[test]
fn the_host_chooses_capabilities_and_receives_script_values() {
    let mut engine = Interpreter::new();
    engine.define_function("document_title", 0, |_| Ok("Example".into()));
    engine.enable_standard_module("json").unwrap();

    let result = engine
        .run_source(
            "import json\nreturn json.stringify({title: document_title()})",
            "embedded.kr",
        )
        .unwrap();
    assert_eq!(result, Value::from(r#"{"title":"Example"}"#));

    for import in [
        "import process",
        "import fs",
        r#"import "./private.kr" as private"#,
    ] {
        let error = engine.run_source(import, "embedded.kr").unwrap_err();
        assert_eq!(error.kind, "ImportError");
        assert_eq!(error.location.unwrap().line, 1);
    }
}

#[test]
fn conditions_require_booleans_and_integer_overflow_is_reported() {
    let engine = Interpreter::new();
    let error = engine
        .run_source("if 1:\n    pass", "types.kr")
        .unwrap_err();
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.location.unwrap().column, 4);

    let value = engine
        .run_source(
            "try:\n    x = 9223372036854775807 + 1\ncatch err:\n    return err.kind",
            "overflow.kr",
        )
        .unwrap();
    assert_eq!(value, Value::from("OverflowError"));
}

#[test]
fn json_preserves_large_integers_and_rejects_values_outside_the_integer_range() {
    let mut engine = Interpreter::new();
    engine.enable_standard_module("json").unwrap();
    let result = engine.run_source(
        "import json\nreturn json.stringify(json.parse('[-9223372036854775808,9223372036854775807,1.5]'))",
        "json.kr",
    ).unwrap();
    assert_eq!(
        result,
        Value::from("[-9223372036854775808,9223372036854775807,1.5]")
    );

    let error = engine
        .run_source(
            "import json\nreturn json.parse('-9223372036854775809')",
            "json.kr",
        )
        .unwrap_err();
    assert_eq!(error.kind, "JsonError");
}

#[test]
fn command_errors_are_catchable_and_keep_the_process_output() {
    let result = Interpreter::cli()
        .run_source(
            r#"
import process
try:
    process.run("sh", ["-c", "printf output; printf failed >&2; exit 7"])
catch err:
    return [err.kind, err.exit_code, err.stdout, err.stderr]
"#,
            "command.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "ProcessError".into(),
            7.into(),
            "output".into(),
            "failed".into()
        ])
    );
}

#[test]
fn cli_preloads_modules_and_hosts_opt_in() {
    let cli = Interpreter::cli();
    let value = cli
        .run_source(
            r#"return [json.parse("[1,2]"), fs.read_text, process.run, http.get, env.get("PATH")]"#,
            "prelude.kr",
        )
        .unwrap();
    let Value::List(values) = value else {
        panic!("expected a list")
    };
    let values = values.values();
    assert_eq!(values[0], Value::list(vec![1.into(), 2.into()]));
    assert!(
        values[1..4]
            .iter()
            .all(|value| matches!(value, Value::Function(_)))
    );
    assert!(matches!(values[4], Value::String(_)));

    let mut host = Interpreter::new();
    for module in ["fs", "env", "http", "process"] {
        let error = host
            .run_source(&format!("return {module}"), "host.kr")
            .unwrap_err();
        assert_eq!(error.kind, "NameError");
    }
    host.add_module("process", Default::default());
    for source in [r#"$ "printf no""#, r#"return $ "printf no""#] {
        let error = host.run_source(source, "host.kr").unwrap_err();
        assert_eq!(error.kind, "ProcessError");
        assert!(
            error
                .message
                .contains("require the standard process module")
        );
    }
    host.enable_standard_module("process").unwrap();
    let value = host
        .run_source(r#"return ($ "printf ok").out"#, "host.kr")
        .unwrap();
    assert_eq!(value, Value::from("ok"));
}

#[test]
fn captures_results_interpolates_exact_arguments_and_splits_lines() {
    let value = Interpreter::cli()
        .run_source(
            r#"
argument = "a b; $(echo unintended)"
out = $ "printf '<%s>' {argument}"
lines_out = `printf '%s\n' wifi wired`
failed = $ "sh -c 'printf output; printf failed >&2; exit 7'"
return [out.out, out.ok, lines_out.lines(), failed.out, failed.err, failed.code, failed.ok]
"#,
            "command.kr",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![
            "<a b; $(echo unintended)>".into(),
            Value::Bool(true),
            Value::list(vec!["wifi".into(), "wired".into()]),
            "output".into(),
            "failed".into(),
            7.into(),
            Value::Bool(false),
        ])
    );
}

#[test]
fn command_pipelines_and_output_iteration_work() {
    let value = Interpreter::cli()
        .run_source(
            r#"
needle = "wifi"
matches = $`printf '%s\n' wifi-main wired wifi-guest` -> lines() -> filter(x => x contains needle)
output = $ "printf '%s\n' wifi-main wired"
labels = []
for line in output.lines():
    if "wifi" in line:
        labels.append("Found module: {line}")
return [matches, labels, '\{line}', "\{line}", "\r\none\r\ntwo\n".lines()]
"#,
            "pipeline.kr",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![
            Value::list(vec!["wifi-main".into(), "wifi-guest".into()]),
            Value::list(vec!["Found module: wifi-main".into()]),
            "{line}".into(),
            "{line}".into(),
            Value::list(vec!["".into(), "one".into(), "two".into()]),
        ])
    );
}

#[test]
fn pipeline_evaluates_input_before_stage_and_prepends_it() {
    let value = Interpreter::new()
        .run_source(
            r#"
log = []
fn input():
    log.append("input")
    return "a"

fn join(a, b):
    return a + b

fn stage():
    log.append("stage")
    return join

fn arg():
    log.append("arg")
    return "b"

result = input() -> stage()(arg()) -> len
return [result, log]
"#,
            "order.kr",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![
            2.into(),
            Value::list(vec!["input".into(), "stage".into(), "arg".into()]),
        ])
    );
}

#[test]
fn http_returns_the_status_headers_and_body_for_local_get_and_post() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/checks", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for (method, body) in [("GET", ""), ("POST", r#"{"ok":true}"#)] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = BufReader::new(&mut stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(
                headers.starts_with(&format!("{method} /checks ")),
                "{headers}"
            );
            assert!(
                headers.to_ascii_lowercase().contains("x-kiro: test"),
                "{headers}"
            );
            let mut received = vec![0; body.len()];
            request.read_exact(&mut received).unwrap();
            assert_eq!(received, body.as_bytes());
            stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nX-Reply: one\r\nX-Reply: two\r\nConnection: close\r\n\r\ngone",
            ).unwrap();
        }
    });

    let mut host = Interpreter::new();
    host.enable_standard_module("http").unwrap();
    host.enable_standard_module("json").unwrap();
    host.set_global("url", Value::from(url));
    let result = host
        .run_source(
            r#"
headers = {"x-kiro": "test", "content-type": "application/json"}
first = http.get(url, headers)
second = http.post(url, json.stringify({ok: true}), headers)
return [first.status, first.ok, first.headers["x-reply"], second.body]
"#,
            "http.kr",
        )
        .unwrap();
    server.join().unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            404.into(),
            Value::Bool(false),
            Value::list(vec!["one".into(), "two".into()]),
            "gone".into(),
        ])
    );
}

#[test]
fn interpolates_expressions_in_the_current_scope() {
    let value = Interpreter::new()
        .run_source(
            r#"
fn summarize(err, matches):
    return "Ollama check failed ({err.kind}): {err.message}. Saved {len(matches)} matches."
summary = summarize({kind: "HttpError", message: "offline"}, ["a", "b"])
return [summary, "{({key: "a}b"}).key}", "{len(["a", "{3 + 4}"])}", "\{len(matches)}"]
"#,
            "interpolation.kr",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![
            "Ollama check failed (HttpError): offline. Saved 2 matches.".into(),
            "a}b".into(),
            "2".into(),
            "{len(matches)}".into(),
        ])
    );
}

#[test]
fn command_interpolation_evaluates_once_and_keeps_each_argument_intact() {
    let value = Interpreter::cli()
        .run_source(
            r#"
calls = []
fn argument():
    calls.append("called")
    return "a b; $(echo unintended)"

out = $ "printf '<%s>' {argument()} {len([1, 2])} '\{1 + 2}'"
nested = `printf '%s' {{key: "text with } and spaces"}.key}`
return [out.out, nested.out, len(calls)]
"#,
            "command-interpolation.kr",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![
            "<a b; $(echo unintended)><2><{1 + 2}>".into(),
            "text with } and spaces".into(),
            1.into(),
        ])
    );
}

#[test]
fn invalid_interpolations_report_the_original_source_location() {
    let engine = Interpreter::new();
    let error = engine
        .run_source("\nreturn \"x {len([1, 2]) 3}\"", "trailing.kr")
        .unwrap_err();
    assert_eq!(error.kind, "SyntaxError");
    let location = error.location.unwrap();
    assert_eq!(location.path.to_str(), Some("trailing.kr"));
    assert_eq!((location.line, location.column), (2, 24));

    let error = engine
        .run_source("\nreturn \"x {missing.name}\"", "missing.kr")
        .unwrap_err();
    assert_eq!(error.kind, "NameError");
    let location = error.location.unwrap();
    assert_eq!((location.line, location.column), (2, 12));
}

#[test]
fn json_stringify_accepts_an_optional_pretty_boolean() {
    let mut engine = Interpreter::new();
    engine.enable_standard_module("json").unwrap();
    let result = engine
        .run_source(
            "value = {items: [1, 2]}\nreturn [json.stringify(value), json.stringify(value, false), json.stringify(value, true)]",
            "json-format.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            r#"{"items":[1,2]}"#.into(),
            r#"{"items":[1,2]}"#.into(),
            "{\n  \"items\": [\n    1,\n    2\n  ]\n}".into(),
        ])
    );
    for source in [
        "return json.stringify(1, 2)",
        "return json.stringify()",
        "return json.stringify(1, true, false)",
    ] {
        let error = engine.run_source(source, "json-format.kr").unwrap_err();
        assert_eq!(error.kind, "TypeError");
    }
}

#[test]
fn http_connection_setup_times_out_before_the_full_request_budget() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let (release, wait) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.read_exact(&mut [0]).unwrap();
        // Keep the TLS handshake open long enough to distinguish the connection budget.
        let _ = wait.recv_timeout(Duration::from_secs(5));
    });

    let mut host = Interpreter::new();
    host.enable_standard_module("http").unwrap();
    host.set_global("url", url.into());
    let result = host.run_source("return http.get(url)", "connect-timeout.kr");
    let _ = release.send(());
    server.join().unwrap();

    let error = result.unwrap_err();
    assert_eq!(error.kind, "HttpError");
    assert!(
        error.message.to_ascii_lowercase().contains("timeout"),
        "{error}"
    );
}

#[test]
fn pipelines_transform_reduce_sort_and_deduplicate_values() {
    let engine = Interpreter::new();
    let result = engine.run_source(
        r#"
sums = [3, 1, 3, 2] -> unique() -> sort() -> map(x => x * 2) -> reduce(0, (acc, x) => acc + x)
records = [{id: "c", rank: 2}, {id: "a", rank: 1}, {id: "b", rank: 1}]
ordered = records -> sort(row => row.rank) -> map(row => row.id)
fn any_test(x):
    if x == 1:
        return true
    return missing
fn all_test(x):
    if x == 1:
        return false
    return missing
return [sums, ordered, any([1, 2], any_test), all([1, 2], all_test), any([], x => true), all([], x => false)]
"#,
        "pipelines.kr",
    ).unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            12.into(),
            Value::list(vec!["a".into(), "b".into(), "c".into()]),
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
        ])
    );
    let error = engine
        .run_source(r#"return sort([1, "x"])"#, "sort.kr")
        .unwrap_err();
    assert_eq!(error.kind, "TypeError");
}

#[test]
fn string_methods_compose_with_the_pipeline() {
    let result = Interpreter::new()
        .run_source(
            r#"
text = "  WLAN, USB, Lan  ".trim().replace(", ", ",").lower()
parts = text.split(",")
joined = "-".join(parts -> map(part => part.upper()))
return [joined, text.starts_with("wlan"), text.ends_with("lan"), ".".join([])]
"#,
            "strings.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "WLAN-USB-LAN".into(),
            Value::Bool(true),
            Value::Bool(true),
            "".into(),
        ])
    );
}

#[test]
fn retry_returns_a_success_or_the_last_error_and_exit_is_separate() {
    use std::cell::Cell;
    use std::rc::Rc;

    let mut engine = Interpreter::new();
    let counter = Rc::new(Cell::new(0));
    let calls = counter.clone();
    engine.define_function("transient", 0, move |_| {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            return Ok(17.into());
        }
        Err(kiro::Error::new("TestError", calls.get().to_string()))
    });
    let result = engine
        .run_source(
            r#"
first = retry(3, 0, transient)
try:
    retry(2, 0, transient)
catch err:
    last = err.message
return [first, last]
"#,
            "retry.kr",
        )
        .unwrap();
    assert_eq!(result, Value::list(vec![17.into(), "5".into()]));
    assert_eq!(counter.get(), 5);
    let signal = engine
        .run_source("retry(2, 0, () => exit(7))", "exit.kr")
        .unwrap_err();
    assert_eq!(signal.exit_code(), Some(7));
}

#[test]
fn parallel_overlaps_tasks_and_returns_in_input_order() {
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    let mut host = Interpreter::new();
    let sync = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let calls = sync.clone();
    host.define_thread_safe_function("overlap", 1, move |args| {
        let (lock, barrier) = &*calls;
        let mut threads = lock.lock().unwrap();
        threads.push(std::thread::current().id());
        barrier.notify_all();
        let (guard, timeout) = barrier
            .wait_timeout_while(threads, Duration::from_secs(2), |seen| seen.len() < 2)
            .unwrap();
        drop(guard);
        if timeout.timed_out() {
            return Err(kiro::Error::new("TestError", "tasks did not overlap"));
        }
        if args[0].as_int()? == 1 {
            std::thread::sleep(Duration::from_millis(30));
        }
        Ok(args[0].clone())
    });
    let outcome = host
        .run_source(
            r#"
prefix = 10
fn make():
    result = overlap(1) + prefix
    return () => result
values = parallel([make, () => overlap(2)])
return [values[0](), values[1], parallel([])]
"#,
            "parallel.kr",
        )
        .unwrap();
    assert_eq!(
        outcome,
        Value::list(vec![11.into(), 2.into(), Value::list(Vec::new())])
    );
    let recorded = sync.0.lock().unwrap();
    assert_ne!(recorded[0], recorded[1]);
    assert!(!recorded.contains(&std::thread::current().id()));
}

#[test]
fn time_formats_utc_and_sleeps_in_milliseconds() {
    let mut host = Interpreter::new();
    host.enable_standard_module("time").unwrap();
    let start = std::time::Instant::now();
    let result = host
        .run_source(
            r#"
before = time.now()
time.sleep(10)
return [time.now() >= before, time.format(0), time.format(-1, "%Y-%m-%d %H:%M:%S%.3f")]
"#,
            "time.kr",
        )
        .unwrap();
    assert!(start.elapsed() >= std::time::Duration::from_millis(10));
    assert_eq!(
        result,
        Value::list(vec![
            Value::Bool(true),
            "1970-01-01T00:00:00.000Z".into(),
            "1969-12-31 23:59:59.999".into(),
        ])
    );
    assert_eq!(
        host.run_source(r#"time.format(0, "%Q")"#, "time.kr")
            .unwrap_err()
            .kind,
        "ValueError"
    );
}

#[test]
fn interpreter_environments_are_isolated() {
    let mut first = Interpreter::new();
    first.enable_standard_module("env").unwrap();
    let mut second = Interpreter::new();
    second.enable_standard_module("env").unwrap();
    let name = "KIRO_TEST_ISOLATED_ENV_37AD6";
    first
        .run_source(&format!(r#"env.set("{name}", "first only")"#), "env.kr")
        .unwrap();
    let get = format!(r#"return env.get("{name}")"#);
    assert_eq!(
        first.run_source(&get, "env.kr").unwrap(),
        Value::from("first only")
    );
    assert_eq!(second.run_source(&get, "env.kr").unwrap(), Value::Null);
    assert!(std::env::var_os(name).is_none());
}

#[test]
fn http_methods_encode_json_bodies_and_parse_response_json() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for (method, body, content_type) in [
            ("POST", r#"{"enabled":true}"#, Some("application/json")),
            ("PUT", "[1,2]", Some("application/example")),
            ("PATCH", "raw text", Some("text/plain; charset=utf-8")),
            ("DELETE", "", None),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut request = BufReader::new(&mut stream);
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            assert_eq!(line, format!("{method} /data HTTP/1.1\r\n"));
            let mut seen_content_type = None;
            loop {
                line.clear();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-type")
                {
                    assert!(seen_content_type.replace(value.trim().to_owned()).is_none());
                }
            }
            assert_eq!(seen_content_type.as_deref(), content_type);
            let mut received = vec![0; body.len()];
            request.read_exact(&mut received).unwrap();
            assert_eq!(received, body.as_bytes());
            let response = format!(r#"{{"method":"{method}"}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        }
    });

    let mut host = Interpreter::new();
    host.enable_standard_module("http").unwrap();
    host.set_global("url", Value::from(url));
    let result = host
        .run_source(
            r#"
return [
    http.post(url, {enabled: true}).json().method,
    http.put(url, [1, 2], {"Content-Type": "application/example"}).json().method,
    http.patch(url, "raw text").json().method,
    http.delete(url).json().method,
]
"#,
            "http-methods.kr",
        )
        .unwrap();
    server.join().unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "POST".into(),
            "PUT".into(),
            "PATCH".into(),
            "DELETE".into()
        ])
    );
}

#[test]
fn parallel_list_appends_cannot_create_a_cycle() {
    let result = Interpreter::new()
        .run_source(
            r#"
left = []
right = []
try:
    parallel([() => left.append(right), () => right.append(left)])
catch err:
    return [err.kind, len(left) + len(right)]
"#,
            "parallel-list.kr",
        )
        .unwrap();
    assert_eq!(result, Value::list(vec!["ValueError".into(), 1.into()]));
}
