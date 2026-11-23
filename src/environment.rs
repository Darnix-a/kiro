use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::runtime::native;
use crate::{Error, Result, Value};

#[derive(Default)]
pub(crate) struct Environment {
    overrides: Mutex<BTreeMap<String, String>>,
}

impl Environment {
    pub(crate) fn overrides(&self) -> BTreeMap<String, String> {
        self.overrides.lock().unwrap().clone()
    }

    pub(crate) fn module(self: &Arc<Self>) -> Value {
        let get = self.clone();
        let set = self.clone();
        let all = self.clone();
        Value::map(BTreeMap::from([
            (
                "get".into(),
                native("env.get", Some(1), move |args| {
                    let name = variable_name(&args[0])?;
                    if let Some(value) = get.overrides.lock().unwrap().get(name).cloned() {
                        return Ok(value.into());
                    }
                    match std::env::var(name) {
                        Ok(value) => Ok(value.into()),
                        Err(std::env::VarError::NotPresent) => Ok(Value::Null),
                        Err(std::env::VarError::NotUnicode(_)) => Err(Error::new(
                            "EnvError",
                            format!("'{name}' is not valid UTF-8"),
                        )),
                    }
                }),
            ),
            (
                "set".into(),
                native("env.set", Some(2), move |args| {
                    let name = variable_name(&args[0])?;
                    let value = args[1].as_str()?;
                    if value.contains('\0') {
                        return Err(Error::new(
                            "EnvError",
                            "environment variable value contains a NUL byte",
                        ));
                    }
                    set.overrides
                        .lock()
                        .unwrap()
                        .insert(name.into(), value.into());
                    Ok(Value::Null)
                }),
            ),
            (
                "all".into(),
                native("env.all", Some(0), move |_| {
                    let mut values: BTreeMap<_, _> = std::env::vars_os()
                        .filter_map(|(name, value)| {
                            Some((
                                name.into_string().ok()?,
                                Value::from(value.into_string().ok()?),
                            ))
                        })
                        .collect();
                    values.extend(
                        all.overrides()
                            .into_iter()
                            .map(|(name, value)| (name, value.into())),
                    );
                    Ok(Value::map(values))
                }),
            ),
        ]))
    }
}

fn variable_name(value: &Value) -> Result<&str> {
    let name = value.as_str()?;
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(Error::new("EnvError", "invalid environment variable name"));
    }
    Ok(name)
}
