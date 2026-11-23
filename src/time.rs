use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{
    DateTime,
    format::{Item, StrftimeItems},
};

use crate::runtime::native;
use crate::{Error, Result, Value};

pub(crate) fn module() -> Value {
    Value::map(BTreeMap::from([
        (
            "sleep".into(),
            native("time.sleep", Some(1), |args| {
                std::thread::sleep(milliseconds(&args[0])?);
                Ok(Value::Null)
            }),
        ),
        (
            "now".into(),
            native("time.now", Some(0), |_| {
                let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(duration) => duration.as_millis() as i128,
                    Err(error) => -(error.duration().as_millis() as i128),
                };
                let millis = i64::try_from(millis)
                    .map_err(|_| Error::new("ValueError", "timestamp is out of range"))?;
                Ok(Value::Int(millis))
            }),
        ),
        ("format".into(), native("time.format", None, format)),
    ]))
}

fn format(args: &[Value]) -> Result<Value> {
    let (timestamp, pattern) = match args {
        [timestamp] => (timestamp, "%Y-%m-%dT%H:%M:%S%.3fZ"),
        [timestamp, pattern] => (timestamp, pattern.as_str()?),
        _ => {
            return Err(Error::new(
                "TypeError",
                "time.format expects 1 or 2 arguments",
            ));
        }
    };
    let date = DateTime::from_timestamp_millis(timestamp.as_int()?)
        .ok_or_else(|| Error::new("ValueError", "timestamp is out of range"))?;
    let items = StrftimeItems::new(pattern);
    if items.clone().any(|item| matches!(item, Item::Error)) {
        return Err(Error::new("ValueError", "invalid time format"));
    }
    Ok(date.format_with_items(items).to_string().into())
}

pub(crate) fn milliseconds(value: &Value) -> Result<Duration> {
    let ms = u64::try_from(value.as_int()?)
        .map_err(|_| Error::new("ValueError", "milliseconds must be non-negative"))?;
    Ok(Duration::from_millis(ms))
}
