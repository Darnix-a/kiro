use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::Value;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Position {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug)]
pub struct Error {
    pub kind: &'static str,
    pub message: String,
    pub location: Option<Box<Location>>,
    pub details: BTreeMap<String, Value>,
}

impl Error {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            location: None,
            details: BTreeMap::new(),
        }
    }

    pub(crate) fn at(mut self, path: &std::path::Path, position: Position) -> Self {
        if self.location.is_none() {
            self.location = Some(Box::new(Location {
                path: path.to_path_buf(),
                line: position.line,
                column: position.column,
            }));
        }
        self
    }

    pub(crate) fn into_value(self) -> Value {
        let mut fields = self.details;
        fields.insert("kind".into(), self.kind.into());
        fields.insert("message".into(), self.message.into());
        if let Some(location) = self.location {
            fields.insert("path".into(), location.path.display().to_string().into());
            fields.insert("line".into(), (location.line as i64).into());
            fields.insert("column".into(), (location.column as i64).into());
        }
        Value::map(fields)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = &self.location {
            write!(
                formatter,
                "{}:{}:{}: ",
                location.path.display(),
                location.line,
                location.column
            )?;
        }
        write!(formatter, "{}: {}", self.kind, self.message)?;
        if let Some(Value::String(stderr)) = self.details.get("stderr") {
            let stderr = stderr.trim_end();
            if !stderr.is_empty() {
                write!(formatter, "\n{stderr}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
