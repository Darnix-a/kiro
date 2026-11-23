use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::Arc;

use serde_json::{Number, Value as Json};

use crate::environment::Environment;
use crate::runtime::native;
use crate::{Error, Result, Value};
use crate::{command, http};

pub(crate) fn print_function() -> Value {
    native("print", None, |args| {
        let line = args
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(io::stdout().lock(), "{line}")
            .map_err(|error| Error::new("IoError", format!("stdout: {error}")))?;
        Ok(Value::Null)
    })
}

pub(crate) const MODULES: [&str; 10] = [
    "process", "json", "fs", "env", "http", "time", "regex", "path", "math", "random",
];

pub(crate) fn module(name: &str, environment: &Arc<Environment>) -> Option<Value> {
    let functions = match name {
        "process" => {
            let environment = environment.clone();
            BTreeMap::from([(
                "run".into(),
                native("process.run", Some(2), move |args| run(args, &environment)),
            )])
        }
        "http" => return Some(http::module()),
        "env" => return Some(environment.module()),
        "fs" => return Some(crate::filesystem::module()),
        "time" => return Some(crate::time::module()),
        "json" => BTreeMap::from([
            (
                "parse".into(),
                native("json.parse", Some(1), |args| parse_json(args[0].as_str()?)),
            ),
            (
                "stringify".into(),
                native("json.stringify", None, |args| {
                    let (value, pretty) = match args {
                        [value] => (value, false),
                        [value, pretty] => (value, pretty.as_bool()?),
                        _ => {
                            return Err(Error::new(
                                "TypeError",
                                format!(
                                    "json.stringify expects 1 or 2 arguments, got {}",
                                    args.len()
                                ),
                            ));
                        }
                    };
                    Ok(encode_json(value, pretty)?.into())
                }),
            ),
        ]),
        _ => return crate::general::module(name),
    };
    Some(Value::map(functions))
}

fn run(args: &[Value], environment: &Environment) -> Result<Value> {
    let executable = args[0].as_str()?;
    let Value::List(arguments) = &args[1] else {
        return Err(args[1].expected("a list of command arguments"));
    };
    let arguments = arguments.values();
    let strings = arguments
        .iter()
        .map(Value::as_str)
        .collect::<Result<Vec<_>>>()?;
    let output = command::execute(executable, &strings, environment)?;
    let mut fields = BTreeMap::from([
        (
            "exit_code".into(),
            output
                .status
                .code()
                .map_or(Value::Null, |code| (code as i64).into()),
        ),
        (
            "stdout".into(),
            String::from_utf8_lossy(&output.stdout).into_owned().into(),
        ),
        (
            "stderr".into(),
            String::from_utf8_lossy(&output.stderr).into_owned().into(),
        ),
    ]);
    if output.status.success() {
        return Ok(Value::map(fields));
    }
    let mut error = Error::new(
        "ProcessError",
        format!("{executable:?} {strings:?} failed with {}", output.status),
    );
    fields.insert("command".into(), executable.into());
    fields.insert("args".into(), Value::list(arguments));
    error.details = fields;
    Err(error)
}

fn json_error(error: serde_json::Error) -> Error {
    Error::new("JsonError", error.to_string())
}

fn from_json(value: Json) -> Result<Value> {
    Ok(match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(value),
        Json::String(value) => value.into(),
        Json::Number(value) => {
            if value.as_str().contains(['.', 'e', 'E']) {
                Value::Float(value.as_f64().ok_or_else(|| {
                    Error::new("JsonError", "number is outside the floating-point range")
                })?)
            } else {
                Value::Int(value.as_i64().ok_or_else(|| {
                    Error::new("JsonError", "integer is outside the signed 64-bit range")
                })?)
            }
        }
        Json::Array(values) => {
            Value::list(values.into_iter().map(from_json).collect::<Result<_>>()?)
        }
        Json::Object(values) => Value::map(
            values
                .into_iter()
                .map(|(key, value)| Ok((key, from_json(value)?)))
                .collect::<Result<_>>()?,
        ),
    })
}

pub(crate) fn to_json(value: &Value, depth: usize) -> Result<Json> {
    if depth >= 128 {
        return Err(Error::new("JsonError", "JSON nesting exceeds 128 levels"));
    }
    Ok(match value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::String(value) => Json::String(value.clone()),
        Value::Int(value) => Json::Number(Number::from(*value)),
        Value::Float(value) => {
            Json::Number(Number::from_f64(*value).ok_or_else(|| {
                Error::new("JsonError", "non-finite numbers cannot be serialized")
            })?)
        }
        Value::List(values) => Json::Array(
            values
                .values()
                .iter()
                .map(|value| to_json(value, depth + 1))
                .collect::<Result<_>>()?,
        ),
        Value::Map(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), to_json(value, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Struct(value) => Json::Object(
            value
                .fields()
                .iter()
                .map(|(key, value)| Ok((key.clone(), to_json(value, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Enum(value) => Json::String(value.member().into()),
        Value::Function(_) => {
            return Err(Error::new("JsonError", "functions cannot be serialized"));
        }
        Value::Command(result) => {
            let fields = ["out", "err", "code", "ok"]
                .into_iter()
                .map(|key| Ok((key.to_owned(), to_json(&result.field(key)?, depth + 1)?)))
                .collect::<Result<_>>()?;
            Json::Object(fields)
        }
    })
}

pub(crate) fn input_function() -> Value {
    native("input", None, |args| {
        let prompt = match args {
            [] => "",
            [prompt] => prompt.as_str()?,
            _ => return Err(Error::new("TypeError", "input expects 0 or 1 arguments")),
        };
        let io_error = |error: io::Error| Error::new("IoError", error.to_string());
        if !prompt.is_empty() {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(prompt.as_bytes())
                .and_then(|()| stdout.flush())
                .map_err(io_error)?;
        }
        let mut text = String::new();
        if io::stdin().read_line(&mut text).map_err(io_error)? == 0 {
            return Ok(Value::Null);
        }
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        Ok(text.into())
    })
}

pub(crate) fn parse_json(text: &str) -> Result<Value> {
    from_json(serde_json::from_str(text).map_err(json_error)?)
}

pub(crate) fn encode_json(value: &Value, pretty: bool) -> Result<String> {
    let value = to_json(value, 0)?;
    if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    }
    .map_err(json_error)
}

type StringMethod = fn(&str, &[Value]) -> Result<Value>;

pub(crate) fn string_method(name: &str) -> Option<(usize, StringMethod)> {
    Some(match name {
        "trim" => (0, |text, _| Ok(text.trim().into())),
        "lower" => (0, |text, _| Ok(text.to_lowercase().into())),
        "upper" => (0, |text, _| Ok(text.to_uppercase().into())),
        "split" => (1, |text, args| {
            let separator = args[0].as_str()?;
            if separator.is_empty() {
                return Err(Error::new("ValueError", "split separator cannot be empty"));
            }
            Ok(Value::list(
                text.split(separator).map(Value::from).collect(),
            ))
        }),
        "replace" => (2, |text, args| {
            Ok(text.replace(args[0].as_str()?, args[1].as_str()?).into())
        }),
        "starts_with" => (1, |text, args| {
            Ok(Value::Bool(text.starts_with(args[0].as_str()?)))
        }),
        "ends_with" => (1, |text, args| {
            Ok(Value::Bool(text.ends_with(args[0].as_str()?)))
        }),
        "join" => (1, |separator, args| {
            let Value::List(values) = &args[0] else {
                return Err(args[0].expected("a list of strings"));
            };
            let values = values.values();
            let strings = values
                .iter()
                .map(Value::as_str)
                .collect::<Result<Vec<_>>>()?;
            Ok(strings.join(separator).into())
        }),
        _ => return None,
    })
}
