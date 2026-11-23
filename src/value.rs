use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::runtime::Callable;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(List),
    Map(Rc<BTreeMap<String, Value>>),
    Function(Rc<Callable>),
}

#[derive(Clone, Debug)]
pub struct List(Rc<RefCell<Vec<Value>>>);

impl List {
    pub fn values(&self) -> Vec<Value> {
        self.0.borrow().clone()
    }

    pub(crate) fn append(&self, value: Value) -> Result<()> {
        if value.contains_list(self, &mut HashSet::new()) {
            return Err(Error::new("ValueError", "a list cannot contain itself"));
        }
        self.0.borrow_mut().push(value);
        Ok(())
    }
}

impl Value {
    pub fn list(values: Vec<Value>) -> Self {
        Self::List(List(Rc::new(RefCell::new(values))))
    }

    pub fn map(values: BTreeMap<String, Value>) -> Self {
        Self::Map(Rc::new(values))
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
            Self::List(value) => value.0.borrow().len(),
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
            other => Err(other.expected("a map")),
        }
    }

    pub(crate) fn index(&self, index: &Self) -> Result<Value> {
        match self {
            Self::Map(_) => self.field(index.as_str()?),
            Self::List(list) => index
                .as_int()
                .map(|i| list.0.borrow().get(i as usize).cloned())?
                .ok_or_else(|| Error::new("IndexError", "list index out of range")),
            Self::String(text) => index
                .as_int()
                .map(|i| text.chars().nth(i as usize))?
                .map(|ch| Self::String(ch.to_string()))
                .ok_or_else(|| Error::new("IndexError", "string index out of range")),
            other => Err(other.expected("a list, string, or map")),
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
            other => Err(other.expected("a list, string, or map")),
        }
    }

    fn contains_list(&self, target: &List, seen: &mut HashSet<*const ()>) -> bool {
        match self {
            Self::List(list) => {
                Rc::ptr_eq(&list.0, &target.0)
                    || (seen.insert(Rc::as_ptr(&list.0).cast())
                        && list
                            .0
                            .borrow()
                            .iter()
                            .any(|value| value.contains_list(target, seen)))
            }
            Self::Map(map) => {
                seen.insert(Rc::as_ptr(map).cast())
                    && map.values().any(|value| value.contains_list(target, seen))
            }
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
                for (index, value) in list.0.borrow().iter().enumerate() {
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
            (Self::List(a), Self::List(b)) => *a.0.borrow() == *b.0.borrow(),
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::Function(a), Self::Function(b)) => Rc::ptr_eq(a, b),
            _ => self.compare(other) == Some(Ordering::Equal),
        }
    }
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
