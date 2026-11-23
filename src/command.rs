use std::process::{Command, ExitStatus, Output};

use crate::environment::Environment;
use crate::{Error, Result, Value};

/// Captured output from a command literal.
#[derive(Clone, Debug, PartialEq)]
pub struct CmdResult {
    pub out: String,
    pub err: String,
    pub code: Option<i64>,
    pub ok: bool,
}

impl CmdResult {
    pub(crate) fn from_output(output: Output) -> Self {
        Self {
            out: String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().map(i64::from),
            ok: output.status.success(),
        }
    }

    pub(crate) fn field(&self, name: &str) -> Result<Value> {
        match name {
            "out" => Ok(self.out.as_str().into()),
            "err" => Ok(self.err.as_str().into()),
            "code" => Ok(self.code.map_or(Value::Null, Value::Int)),
            "ok" => Ok(Value::Bool(self.ok)),
            _ => Err(Error::new("KeyError", format!("no field named '{name}'"))),
        }
    }
}

pub(crate) fn execute(
    executable: &str,
    args: &[&str],
    environment: &Environment,
) -> Result<Output> {
    Command::new(executable)
        .args(args)
        .envs(environment.overrides())
        .output()
        .map_err(|error| {
            process_error(
                format!("could not start {executable:?}: {error}"),
                executable,
                args,
                None,
            )
        })
}

pub(crate) fn stream(executable: &str, args: &[&str], environment: &Environment) -> Result<()> {
    let status = Command::new(executable)
        .args(args)
        .envs(environment.overrides())
        .status()
        .map_err(|error| {
            process_error(
                format!("could not start {executable:?}: {error}"),
                executable,
                args,
                None,
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(process_error(
            format!("{executable:?} {args:?} failed with {status}"),
            executable,
            args,
            Some(status),
        ))
    }
}

fn process_error(
    message: String,
    executable: &str,
    args: &[&str],
    status: Option<ExitStatus>,
) -> Error {
    let mut error = Error::new("ProcessError", message);
    error.details.extend([
        ("command".into(), executable.into()),
        (
            "args".into(),
            Value::list(args.iter().map(|arg| (*arg).into()).collect()),
        ),
        (
            "exit_code".into(),
            status
                .and_then(|status| status.code())
                .map_or(Value::Null, |code| Value::Int(code.into())),
        ),
        ("stdout".into(), "".into()),
        ("stderr".into(), "".into()),
    ]);
    error
}
