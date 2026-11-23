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
