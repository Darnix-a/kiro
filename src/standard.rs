use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use serde_json::{Number, Value as Json};

use crate::runtime::{file_error, native};
use crate::{Error, Result, Value};

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

pub(crate) fn modules() -> BTreeMap<String, Value> {
    ["process", "json", "fs"]
        .into_iter()
        .map(|name| (name.into(), module(name).unwrap()))
        .collect()
}

pub(crate) fn module(name: &str) -> Option<Value> {
    let functions = match name {
        "process" => BTreeMap::from([("run".into(), native("process.run", Some(2), run))]),
        "json" => BTreeMap::from([
            (
                "parse".into(),
                native("json.parse", Some(1), |args| {
                    let value = serde_json::from_str(args[0].as_str()?).map_err(json_error)?;
                    from_json(value)
                }),
            ),
            (
                "stringify".into(),
                native("json.stringify", Some(1), |args| {
                    let value = to_json(&args[0], 0)?;
                    Ok(serde_json::to_string(&value).map_err(json_error)?.into())
                }),
            ),
        ]),
        "fs" => BTreeMap::from([
            (
                "read_text".into(),
                native("fs.read_text", Some(1), |args| {
                    let path = Path::new(args[0].as_str()?);
                    Ok(fs::read_to_string(path)
                        .map_err(|error| file_error(path, error))?
                        .into())
                }),
            ),
            (
                "write_text".into(),
                native("fs.write_text", Some(2), |args| {
                    let path = Path::new(args[0].as_str()?);
                    fs::write(path, args[1].as_str()?).map_err(|error| file_error(path, error))?;
                    Ok(Value::Null)
                }),
            ),
        ]),
        _ => return None,
    };
    Some(Value::map(functions))
}

fn run(args: &[Value]) -> Result<Value> {
    let executable = args[0].as_str()?;
    let Value::List(arguments) = &args[1] else {
        return Err(args[1].expected("a list of command arguments"));
    };
    let arguments = arguments.values();
    let strings = arguments
        .iter()
        .map(Value::as_str)
        .collect::<Result<Vec<_>>>()?;
    let output = Command::new(executable)
        .args(&strings)
        .output()
        .map_err(|error| {
            let mut error = Error::new(
                "ProcessError",
                format!("could not start {executable:?}: {error}"),
            );
            error.details = BTreeMap::from([
                ("command".into(), executable.into()),
                ("args".into(), Value::list(arguments.clone())),
                ("exit_code".into(), Value::Null),
                ("stdout".into(), "".into()),
                ("stderr".into(), "".into()),
            ]);
            error
        })?;
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

fn to_json(value: &Value, depth: usize) -> Result<Json> {
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
        Value::Function(_) => {
            return Err(Error::new("JsonError", "functions cannot be serialized"));
        }
    })
}
