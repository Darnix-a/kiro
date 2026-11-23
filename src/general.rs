use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::runtime::native;
use crate::{Error, Result, Value};

pub(crate) fn module(name: &str) -> Option<Value> {
    let functions = match name {
        "regex" => BTreeMap::from([
            (
                "match".into(),
                native("regex.match", Some(2), |a| {
                    Ok(Value::Bool(regex(&a[0])?.is_match(a[1].as_str()?)))
                }),
            ),
            (
                "find_all".into(),
                native("regex.find_all", Some(2), |a| {
                    let pattern = regex(&a[0])?;
                    Ok(Value::list(
                        pattern
                            .find_iter(a[1].as_str()?)
                            .map(|entry| entry.as_str().into())
                            .collect(),
                    ))
                }),
            ),
            (
                "replace".into(),
                native("regex.replace", Some(3), |a| {
                    Ok(regex(&a[0])?
                        .replace_all(a[1].as_str()?, a[2].as_str()?)
                        .into_owned()
                        .into())
                }),
            ),
        ]),
        "path" => BTreeMap::from([
            (
                "join".into(),
                native("path.join", None, |args| {
                    let mut path = PathBuf::new();
                    for part in args {
                        path.push(part.as_str()?);
                    }
                    Ok(path.to_string_lossy().into_owned().into())
                }),
            ),
            (
                "basename".into(),
                native("path.basename", Some(1), |a| {
                    Ok(Path::new(a[0].as_str()?)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("")
                        .into())
                }),
            ),
            (
                "dirname".into(),
                native("path.dirname", Some(1), |a| {
                    let path = Path::new(a[0].as_str()?);
                    let parent = path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or_else(|| {
                            if path.has_root() {
                                path
                            } else {
                                Path::new(".")
                            }
                        });
                    Ok(parent.to_string_lossy().into_owned().into())
                }),
            ),
            (
                "ext".into(),
                native("path.ext", Some(1), |a| {
                    Ok(Path::new(a[0].as_str()?)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("")
                        .into())
                }),
            ),
        ]),
        "math" => BTreeMap::from([
            (
                "floor".into(),
                native("math.floor", Some(1), |a| round(&a[0], f64::floor)),
            ),
            (
                "ceil".into(),
                native("math.ceil", Some(1), |a| round(&a[0], f64::ceil)),
            ),
            (
                "round".into(),
                native("math.round", Some(1), |a| round(&a[0], f64::round)),
            ),
            (
                "abs".into(),
                native("math.abs", Some(1), |a| match a[0] {
                    Value::Int(value) => value.checked_abs().map(Value::Int).ok_or_else(overflow),
                    _ => Ok(Value::Float(number(&a[0])?.abs())),
                }),
            ),
            (
                "sqrt".into(),
                native("math.sqrt", Some(1), |a| {
                    let value = number(&a[0])?;
                    if value < 0.0 {
                        return Err(Error::new(
                            "ValueError",
                            "square root requires a nonnegative number",
                        ));
                    }
                    Ok(Value::Float(value.sqrt()))
                }),
            ),
            (
                "min".into(),
                native("math.min", None, |a| extreme(a, std::cmp::Ordering::Less)),
            ),
            (
                "max".into(),
                native("math.max", None, |a| {
                    extreme(a, std::cmp::Ordering::Greater)
                }),
            ),
        ]),
        "random" => BTreeMap::from([
            (
                "int".into(),
                native("random.int", Some(2), |a| {
                    let (min, max) = (a[0].as_int()?, a[1].as_int()?);
                    if min > max {
                        return Err(Error::new("ValueError", "random.int expects min <= max"));
                    }
                    Ok(Value::Int(rand::random_range(min..=max)))
                }),
            ),
            (
                "float".into(),
                native("random.float", Some(0), |_| {
                    Ok(Value::Float(rand::random::<f64>()))
                }),
            ),
            (
                "choice".into(),
                native("random.choice", Some(1), |a| {
                    let Value::List(list) = &a[0] else {
                        return Err(a[0].expected("a list"));
                    };
                    let items = list.values();
                    if items.is_empty() {
                        return Err(Error::new(
                            "ValueError",
                            "random.choice requires a nonempty list",
                        ));
                    }
                    Ok(items[rand::random_range(0..items.len())].clone())
                }),
            ),
        ]),
        _ => return None,
    };
    Some(Value::map(functions))
}

fn regex(value: &Value) -> Result<Regex> {
    Regex::new(value.as_str()?).map_err(|error| Error::new("RegexError", error.to_string()))
}

fn number(value: &Value) -> Result<f64> {
    match value {
        Value::Int(value) => Ok(*value as f64),
        Value::Float(value) if value.is_finite() => Ok(*value),
        _ => Err(value.expected("a finite number")),
    }
}

fn round(value: &Value, operation: fn(f64) -> f64) -> Result<Value> {
    if let Value::Int(number) = value {
        return Ok(Value::Int(*number));
    }
    let result = operation(number(value)?);
    if result < i64::MIN as f64 || result >= 9_223_372_036_854_775_808.0 {
        return Err(overflow());
    }
    Ok(Value::Int(result as i64))
}

fn overflow() -> Error {
    Error::new("OverflowError", "integer is out of range")
}

fn extreme(args: &[Value], preferred: std::cmp::Ordering) -> Result<Value> {
    let list;
    let items = if let [Value::List(values)] = args {
        list = values.values();
        list.as_slice()
    } else {
        args
    };
    let Some((first, rest)) = items.split_first() else {
        return Err(Error::new("ValueError", "expected at least one number"));
    };
    number(first)?;
    let mut result = first;
    for value in rest {
        number(value)?;
        if value.compare(result) == Some(preferred) {
            result = value;
        }
    }
    Ok(result.clone())
}

pub(crate) fn range(args: &[Value]) -> Result<Value> {
    let (start, end, step) = match args {
        [start, end] => (start.as_int()?, end.as_int()?, 1),
        [start, end, step] => (start.as_int()?, end.as_int()?, step.as_int()?),
        _ => return Err(Error::new("TypeError", "range expects 2 or 3 arguments")),
    };
    if step == 0 {
        return Err(Error::new("ValueError", "range step must be nonzero"));
    }
    let (start, end, step) = (i128::from(start), i128::from(end), i128::from(step));
    let distance = (end - start) * step.signum();
    let size = if distance <= 0 {
        0
    } else {
        (distance - 1) / step.abs() + 1
    };
    if size > 1_000_000 {
        return Err(Error::new(
            "ValueError",
            "range exceeds one million elements",
        ));
    }
    Ok(Value::list(
        (0..size)
            .map(|index| Value::Int((start + index * step) as i64))
            .collect(),
    ))
}
