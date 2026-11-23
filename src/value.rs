use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::CmdResult;
use crate::error::{Error, Result};
use crate::runtime::{Callable, EnumValue, StructValue};

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(List),
    Map(Arc<BTreeMap<String, Value>>),
    Struct(Arc<StructValue>),
    Enum(Arc<EnumValue>),
    Command(Arc<CmdResult>),
    Function(Arc<Callable>),
}

#[derive(Clone, Debug)]
pub struct List(Arc<Mutex<Vec<Value>>>);

impl List {
    pub fn values(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn append(&self, value: Value) -> Result<()> {
        // Serialize the cycle check and mutation across lists.
        static MUTATIONS: Mutex<()> = Mutex::new(());
        let _mutation = MUTATIONS.lock().unwrap();
        if value.contains_list(self, &mut HashSet::new()) {
            return Err(Error::new("ValueError", "a list cannot contain itself"));
        }
        self.0.lock().unwrap().push(value);
        Ok(())
    }
}

impl Value {
    pub fn list(values: Vec<Value>) -> Self {
        Self::List(List(Arc::new(Mutex::new(values))))
    }

    pub fn map(values: BTreeMap<String, Value>) -> Self {
        Self::Map(Arc::new(values))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
            Self::Float(_) => "float",
            Self::String(_) => "string",
            Self::List(_) => "list",
            Self::Map(_) => "map",
            Self::Struct(_) => "struct",
            Self::Enum(_) => "enum member",
            Self::Command(_) => "command result",
            Self::Function(_) => "function",
        }
    }

    pub fn as_str(&self) -> Result<&str> {
        match self {
            Self::String(value) => Ok(value),
            other => Err(other.expected("a string")),
        }
    }

    pub fn as_int(&self) -> Result<i64> {
        match self {
            Self::Int(value) => Ok(*value),
            other => Err(other.expected("an integer")),
        }
    }

    pub(crate) fn as_bool(&self) -> Result<bool> {
        match self {
            Self::Bool(value) => Ok(*value),
            other => Err(other.expected("a boolean")),
        }
    }

    pub(crate) fn expected(&self, expected: &str) -> Error {
        Error::new(
            "TypeError",
            format!("expected {expected}, got {}", self.type_name()),
        )
    }

    pub(crate) fn length(&self) -> Result<Value> {
        let length = match self {
            Self::String(value) => value.chars().count(),
            Self::List(value) => value.0.lock().unwrap().len(),
            Self::Map(value) => value.len(),
            other => return Err(other.expected("a string, list, or map")),
        };
        Ok(Self::Int(length as i64))
    }

    pub(crate) fn field(&self, name: &str) -> Result<Value> {
        match self {
            Self::Map(fields) => fields
                .get(name)
                .cloned()
                .ok_or_else(|| Error::new("KeyError", format!("no field named '{name}'"))),
            Self::Command(result) => result.field(name),
            Self::Struct(instance) => instance.field(name),
            Self::Enum(member) if name == "name" => Ok(member.member().into()),
            Self::Enum(_) => Err(Error::new("KeyError", format!("no field named '{name}'"))),
            other => Err(other.expected("a map")),
        }
    }

    pub(crate) fn member(&self, name: &str, optional: bool) -> Result<Value> {
        if optional && matches!(self, Self::Null) {
            return Ok(Self::Null);
        }
        match self.field(name) {
            Err(error) if optional && error.kind == "KeyError" => Ok(Self::Null),
            result => result,
        }
    }

    pub(crate) fn index(&self, index: &Self) -> Result<Value> {
        match self {
            Self::Map(_) | Self::Struct(_) | Self::Command(_) => self.field(index.as_str()?),
            Self::List(list) => {
                let index = index.as_int()?;
                let items = list.0.lock().unwrap();
                Ok(items[sequence_index(index, items.len())?].clone())
            }
            Self::String(text) => {
                let index = sequence_index(index.as_int()?, text.chars().count())?;
                Ok(Self::String(text.chars().nth(index).unwrap().to_string()))
            }
            other => Err(other.expected("a list, string, or map")),
        }
    }

    pub(crate) fn slice(&self, start: Option<i64>, end: Option<i64>) -> Result<Value> {
        let bounds = |length| {
            let start = start.map_or(0, |index| clamp_index(index, length));
            let end = end.map_or(length, |index| clamp_index(index, length));
            start..end.max(start)
        };
        match self {
            Self::List(list) => {
                let items = list.0.lock().unwrap();
                Ok(Value::list(items[bounds(items.len())].to_vec()))
            }
            Self::String(text) => {
                let bounds = bounds(text.chars().count());
                Ok(Value::String(
                    text.chars().skip(bounds.start).take(bounds.len()).collect(),
                ))
            }
            other => Err(other.expected("a list or string")),
        }
    }

    pub(crate) fn items(&self) -> Result<Vec<Value>> {
        match self {
            Self::List(list) => Ok(list.values()),
            Self::Map(map) => Ok(map.keys().cloned().map(Self::String).collect()),
            Self::String(text) => Ok(text
                .chars()
                .map(|ch| Self::String(ch.to_string()))
                .collect()),
            Self::Command(result) => Ok(text_lines(&result.out)),
            other => Err(other.expected("a list, string, map, or command result")),
        }
    }

    pub(crate) fn lines(&self) -> Result<Self> {
        match self {
            Self::String(text) => Ok(Self::list(text_lines(text))),
            Self::Command(result) => Ok(Self::list(text_lines(&result.out))),
            other => Err(other.expected("a string or command result")),
        }
    }

    pub(crate) fn contains(&self, needle: &Self) -> Result<Value> {
        let found = match self {
            Self::String(text) => text.contains(needle.as_str()?),
            Self::List(list) => list.values().contains(needle),
            Self::Map(fields) => fields.contains_key(needle.as_str()?),
            other => return Err(other.expected("a string, list, or map")),
        };
        Ok(Self::Bool(found))
    }

    fn contains_list(&self, target: &List, seen: &mut HashSet<*const ()>) -> bool {
        match self {
            Self::List(list) => {
                Arc::ptr_eq(&list.0, &target.0)
                    || (seen.insert(Arc::as_ptr(&list.0).cast())
                        && list
                            .0
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|value| value.contains_list(target, seen)))
            }
            Self::Map(map) => {
                seen.insert(Arc::as_ptr(map).cast())
                    && map.values().any(|value| value.contains_list(target, seen))
            }
            Self::Struct(instance) => {
                seen.insert(Arc::as_ptr(instance).cast())
                    && instance
                        .fields()
                        .values()
                        .any(|value| value.contains_list(target, seen))
            }
            Self::Function(function) => function
                .receiver()
                .is_some_and(|value| value.contains_list(target, seen)),
            _ => false,
        }
    }

    pub(crate) fn compare(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => Some(a.cmp(b)),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(b),
            (Self::Int(a), Self::Float(b)) => compare_integer_float(*a, *b),
            (Self::Float(a), Self::Int(b)) => compare_integer_float(*b, *a).map(Ordering::reverse),
            (Self::String(a), Self::String(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    fn fmt_inner(&self, formatter: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        if depth >= 64 {
            return formatter.write_str("...");
        }
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Int(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value:?}"),
            Self::String(value) if depth == 0 => formatter.write_str(value),
            Self::String(value) => write!(formatter, "{value:?}"),
            Self::List(list) => {
                formatter.write_str("[")?;
                for (index, value) in list.values().iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    value.fmt_inner(formatter, depth + 1)?;
                }
                formatter.write_str("]")
            }
            Self::Map(map) => {
                formatter.write_str("{")?;
                for (index, (key, value)) in map.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{key:?}: ")?;
                    value.fmt_inner(formatter, depth + 1)?;
                }
                formatter.write_str("}")
            }
            Self::Function(function) => write!(formatter, "<fn {}>", function.name()),
            Self::Command(result) => formatter.write_str(&result.out),
            Self::Enum(member) => write!(formatter, "{}.{}", member.enum_name(), member.member()),
            Self::Struct(instance) => {
                write!(formatter, "{}(", instance.name())?;
                for (index, (name, field)) in instance.fields().iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{name}: ")?;
                    field.fmt_inner(formatter, depth + 1)?;
                }
                formatter.write_str(")")
            }
        }
    }
}

fn compare_integer_float(integer: i64, float: f64) -> Option<Ordering> {
    // Compare the integral parts before converting anything to a float.
    if float.is_nan() {
        return None;
    }
    if float >= 9_223_372_036_854_775_808.0 {
        return Some(Ordering::Less);
    }
    if float < i64::MIN as f64 {
        return Some(Ordering::Greater);
    }
    Some(
        integer
            .cmp(&(float as i64))
            .then_with(|| 0.0_f64.partial_cmp(&float.fract()).unwrap()),
    )
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::List(a), Self::List(b)) => a.values() == b.values(),
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::Struct(a), Self::Struct(b)) => a == b,
            (Self::Enum(a), Self::Enum(b)) => a == b,
            (Self::Command(a), Self::Command(b)) => a == b,
            (Self::Function(a), Self::Function(b)) => Arc::ptr_eq(a, b),
            _ => self.compare(other) == Some(Ordering::Equal),
        }
    }
}

fn text_lines(text: &str) -> Vec<Value> {
    text.lines().map(Value::from).collect()
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_inner(formatter, 0)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

fn signed_index(index: i64, length: usize) -> i128 {
    if index < 0 {
        length as i128 + i128::from(index)
    } else {
        i128::from(index)
    }
}

fn sequence_index(index: i64, length: usize) -> Result<usize> {
    let index = signed_index(index, length);
    if (0..length as i128).contains(&index) {
        Ok(index as usize)
    } else {
        Err(Error::new("IndexError", "index out of range"))
    }
}

fn clamp_index(index: i64, length: usize) -> usize {
    signed_index(index, length).clamp(0, length as i128) as usize
}
