use kiro::{Interpreter, Value};

#[test]
fn structs_apply_types_and_fresh_defaults_and_bind_methods() {
    let engine = Interpreter::cli();
    let result = engine
        .run_source(
            r#"
enum Status:
    Active
    Inactive

struct Job:
    title: String
    port: Int = 8080
    status: Status = Status.Active
    tags: List = []
    notes

    fn label(self, prefix = "job"):
        return "{prefix} {self.title}:{self.port}"

first = Job(notes: null, title: "sync")
second = Job(title = "index", port = 80, notes = [], status = Status.Inactive)
first.tags.append("net")
label = first.label
{title, port} = first
return [label(), second.label(prefix: "next"), len(second.tags), title, port, first.status.name,
        first.status != second.status, json.parse(json.stringify(first)).status]
"#,
            "structs.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "job sync:8080".into(),
            "next index:80".into(),
            0.into(),
            "sync".into(),
            8080.into(),
            "Active".into(),
            Value::Bool(true),
            "Active".into(),
        ])
    );

    let error = engine
        .run_source(
            "struct Item:\n    id: Int\nreturn Item(id: \"bad\")",
            "types.kr",
        )
        .unwrap_err();
    assert_eq!(error.kind, "TypeError");
    assert!(error.message.contains("id expects Int"));
}

#[test]
fn matches_literals_or_lists_maps_enums_and_guards_in_order() {
    let result = Interpreter::new()
        .run_source(
            r#"
enum Status:
    Active
    Inactive

fn dispatch(input):
    match input:
        case 1 | 2:
            return "small"
        case [head, ...tail] if head > 2:
            return tail
        case {status: 200, body} if body != "":
            return body
        case Status.Active:
            return "active"
        case _:
            return "other"

previous = "keep"
match {value: "discard"}:
    case {value: previous} if false:
        previous = "wrong"
    case _:
        pass

[a, b] = [4, 5]
{host, port: service_port} = {host: "localhost", port: 88, ignored: true}
return [dispatch(2), dispatch([3, 4, 5]), dispatch({ status: 200, body: "ok" }),
        dispatch(Status.Active), dispatch(false), previous, a+b, host, service_port]
"#,
            "match.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "small".into(),
            Value::list(vec![4.into(), 5.into()]),
            "ok".into(),
            "active".into(),
            "other".into(),
            "keep".into(),
            9.into(),
            "localhost".into(),
            88.into(),
        ])
    );
}

#[test]
fn destructuring_is_atomic_and_or_patterns_bind_the_same_names() {
    let engine = Interpreter::new();
    let result = engine
        .run_source(
            r#"
a = 9
try:
    [a, 1] = [0, 2]
catch err:
    kind = err.kind
return [a, kind]
"#,
            "destructure.kr",
        )
        .unwrap();
    assert_eq!(result, Value::list(vec![9.into(), "PatternError".into()]));
    assert_eq!(
        engine
            .run_source(
                "match []:\n    case [x] | []:\n        pass",
                "bad-pattern.kr"
            )
            .unwrap_err()
            .kind,
        "SyntaxError"
    );
}

#[test]
fn function_defaults_keywords_and_pipeline_arguments_share_binding_rules() {
    let engine = Interpreter::new();
    let result = engine
        .run_source(
            r#"
fallback = 8079
fn connect(host, port = fallback + 1, protocol="http"):
    return "{protocol}://{host}:{port}"
return [connect("localhost"), connect(protocol="https", host: "test", port: 443),
        "host" -> connect(port: 90)]
"#,
            "arguments.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            "http://localhost:8080".into(),
            "https://test:443".into(),
            "http://host:90".into(),
        ])
    );
    for call in ["f(1, x: 2)", "f(unknown: 3)", "f(x: 1, 2)", "f(x: 1, x: 2)"] {
        assert_eq!(
            engine
                .run_source(
                    &format!("fn f(x, y=0):\n    return x\nreturn {call}"),
                    "bad-argument.kr"
                )
                .unwrap_err()
                .kind,
            "TypeError"
        );
    }
}

#[test]
fn comprehensions_slices_and_optional_access_compose() {
    let result = Interpreter::new()
        .run_source(
            r#"
x = 99
labels = ["x{x}" for x in range(0, 5) if x % 2 == 0]
lookup = {label: len(label) for label in labels}
callbacks = [() => x for x in range(0, 2)]
obj = {nested: null}
return [labels[1:4], labels[:1], labels[-1], "ać😺d"[-3:-1], lookup.x2,
        range(5, -1, -2), obj?.nested?.field ?? "default", {}?.missing ?? 7,
        {}?.missing(unknown) ?? false, false ?? unknown, x, callbacks[0]() + callbacks[1]()]
"#,
            "expressions.kr",
        )
        .unwrap();
    assert_eq!(
        result,
        Value::list(vec![
            Value::list(vec!["x2".into(), "x4".into()]),
            Value::list(vec!["x0".into()]),
            "x4".into(),
            "ć😺".into(),
            2.into(),
            Value::list(vec![5.into(), 3.into(), 1.into()]),
            "default".into(),
            7.into(),
            Value::Bool(false),
            Value::Bool(false),
            99.into(),
            1.into(),
        ])
    );
}

#[test]
fn general_purpose_modules_are_preloaded() {
    let engine = Interpreter::cli();
    let result = engine.run_source(r#"
file = path.join("project", "src", "report.txt")
sample = random.float()
return [
    regex.match("[0-9]+", "log7"), regex.find_all("[0-9]+", "log7-23"),
    regex.replace("([0-9]+)", "x1 y2", "[$1]"), path.basename(file), path.dirname(file), path.ext(file),
    math.floor(2.9), math.ceil(2.1), math.round(-2.5), math.abs(-2), math.min([4, 2, 3]),
    math.max(4, 2, 3), math.sqrt(16), random.int(7, 7), sample >= 0.0 and sample < 1.0,
    random.choice(["only"]),
]
"#, "modules.kr").unwrap();
    let dirname = std::path::Path::new("project")
        .join("src")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        result,
        Value::list(vec![
            Value::Bool(true),
            Value::list(vec!["7".into(), "23".into()]),
            "x[1] y[2]".into(),
            "report.txt".into(),
            dirname.into(),
            "txt".into(),
            2.into(),
            3.into(),
            (-3).into(),
            2.into(),
            2.into(),
            4.into(),
            Value::Float(4.0),
            7.into(),
            Value::Bool(true),
            "only".into(),
        ])
    );
    assert_eq!(
        engine
            .run_source("regex.match(\"[\", \"\")", "regex.kr")
            .unwrap_err()
            .kind,
        "RegexError"
    );
    assert_eq!(
        engine
            .run_source("range(1, 3, 0)", "range.kr")
            .unwrap_err()
            .kind,
        "ValueError"
    );
}

#[test]
fn repl_keeps_definitions_and_detects_blocks_and_delimiters() {
    use kiro::Repl;

    assert!(Repl::needs_more_input("fn add(n):\n"));
    assert!(Repl::needs_more_input("fn add(n):\n    return base + n\n"));
    assert!(!Repl::needs_more_input(
        "fn add(n):\n    return base + n\n\n"
    ));
    assert!(Repl::needs_more_input("try:\n    missing\n"));
    assert!(Repl::needs_more_input("[\n    1,\n"));
    assert!(!Repl::needs_more_input("[\n    1,\n]\n"));

    let engine = Interpreter::new();
    let mut repl = engine.repl();
    assert_eq!(repl.eval("base = 40").unwrap(), Value::Null);
    repl.eval("fn add(n):\n    return base + n").unwrap();
    assert_eq!(repl.eval("add(2)").unwrap(), Value::Int(42));
    assert_eq!(repl.eval("missing").unwrap_err().kind, "NameError");
    assert_eq!(repl.eval("add(3)").unwrap(), Value::Int(43));
    assert_eq!(repl.eval("exit(1)").unwrap_err().exit_code(), Some(1));
}
