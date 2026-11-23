use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use crate::error::{Error, Result};
use crate::parser::{
    self, Binary, Block, Expression, ExpressionKind, Function, Import, Statement, StatementKind,
    Unary,
};
use crate::{Value, standard};

type Values = BTreeMap<String, Value>;
type NativeFunction = dyn Fn(&[Value]) -> Result<Value>;

pub struct Callable {
    name: String,
    kind: CallableKind,
}

enum CallableKind {
    Native {
        arity: Option<usize>,
        call: Box<NativeFunction>,
    },
    Script {
        function: Rc<Function>,
        scope: Weak<Scope>,
    },
}

impl Callable {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for Callable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Function").field(&self.name).finish()
    }
}

pub(crate) fn native(
    name: &str,
    arity: Option<usize>,
    call: impl Fn(&[Value]) -> Result<Value> + 'static,
) -> Value {
    Value::Function(Rc::new(Callable {
        name: name.into(),
        kind: CallableKind::Native {
            arity,
            call: Box::new(call),
        },
    }))
}

/// Configures which capabilities a new script execution can access.
pub struct Interpreter {
    globals: Values,
    modules: Values,
    file_imports: bool,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    /// Creates an interpreter with pure helpers and no script-initiated I/O.
    pub fn new() -> Self {
        let mut interpreter = Self {
            globals: Values::new(),
            modules: Values::new(),
            file_imports: false,
        };
        interpreter.define_function("len", 1, |args| args[0].length());
        interpreter
    }

    /// Creates an interpreter with the standard command-line capabilities.
    pub fn cli() -> Self {
        let mut interpreter = Self::new();
        interpreter
            .globals
            .insert("print".into(), standard::print_function());
        interpreter.modules = standard::modules();
        interpreter.allow_file_imports();
        interpreter
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    pub fn define_function(
        &mut self,
        name: impl Into<String>,
        arity: usize,
        call: impl Fn(&[Value]) -> Result<Value> + 'static,
    ) {
        let name = name.into();
        let function = native(&name, Some(arity), call);
        self.set_global(name, function);
    }

    pub fn add_module(&mut self, name: impl Into<String>, exports: Values) {
        self.modules.insert(name.into(), Value::map(exports));
    }

    /// Makes a named standard module available to scripts in this interpreter.
    pub fn enable_standard_module(&mut self, name: &str) -> Result<()> {
        let module = standard::module(name).ok_or_else(|| {
            Error::new("ImportError", format!("unknown standard module '{name}'"))
        })?;
        self.modules.insert(name.into(), module);
        Ok(())
    }

    /// Allows scripts to load source files through relative or absolute imports.
    pub fn allow_file_imports(&mut self) {
        self.file_imports = true;
    }

    pub fn run_file(&self, path: impl AsRef<Path>) -> Result<Value> {
        let path = path.as_ref();
        let canonical = fs::canonicalize(path).map_err(|error| file_error(path, error))?;
        let source = fs::read_to_string(&canonical).map_err(|error| file_error(path, error))?;
        self.run_source(&source, canonical)
    }

    pub fn run_source(&self, source: &str, path: impl AsRef<Path>) -> Result<Value> {
        let path = path.as_ref();
        let program = parser::parse(source, path)?;
        let mut runtime = Runtime {
            config: self,
            modules: BTreeMap::new(),
            loading: BTreeSet::from([path.to_path_buf()]),
            captured_scopes: Vec::new(),
            call_depth: 0,
        };
        let scope = Scope::file(path, &self.globals);
        match runtime.block(&program, &scope)? {
            Flow::Return(value) => Ok(value),
            _ => Ok(Value::Null),
        }
    }
}

struct Scope {
    path: Rc<PathBuf>,
    names: RefCell<Values>,
    exports: RefCell<BTreeSet<String>>,
    local_names: BTreeSet<String>,
    parent: Option<Rc<Scope>>,
    captured: Cell<bool>,
}

impl Scope {
    fn file(path: &Path, globals: &Values) -> Rc<Self> {
        Rc::new(Self {
            path: Rc::new(path.to_path_buf()),
            names: RefCell::new(globals.clone()),
            exports: RefCell::default(),
            local_names: BTreeSet::new(),
            parent: None,
            captured: Cell::new(false),
        })
    }

    fn get(&self, name: &str) -> Result<Value> {
        if let Some(value) = self.names.borrow().get(name) {
            return Ok(value.clone());
        }
        if !self.local_names.contains(name)
            && let Some(parent) = &self.parent
        {
            return parent.get(name);
        }
        Err(Error::new("NameError", format!("'{name}' is not defined")))
    }

    fn assign(&self, name: &str, value: Value, exported: bool) {
        self.names.borrow_mut().insert(name.to_owned(), value);
        if exported {
            self.exports.borrow_mut().insert(name.to_owned());
        }
    }

    fn module(&self) -> Value {
        let names = self.names.borrow();
        Value::map(
            self.exports
                .borrow()
                .iter()
                .map(|name| (name.clone(), names[name].clone()))
                .collect(),
        )
    }
}

enum Flow {
    Next,
    Return(Value),
    Break,
    Continue,
}

struct Runtime<'a> {
    config: &'a Interpreter,
    modules: BTreeMap<PathBuf, Value>,
    loading: BTreeSet<PathBuf>,
    captured_scopes: Vec<Rc<Scope>>,
    call_depth: usize,
}

impl Runtime<'_> {
    fn block(&mut self, statements: &Block, scope: &Rc<Scope>) -> Result<Flow> {
        for statement in statements {
            let flow = self
                .statement(statement, scope)
                .map_err(|error| error.at(&scope.path, statement.at))?;
            if !matches!(flow, Flow::Next) {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }

    fn statement(&mut self, statement: &Statement, scope: &Rc<Scope>) -> Result<Flow> {
        match &statement.kind {
            StatementKind::Assign(name, expression, exported) => {
                let value = self.expression(expression, scope)?;
                scope.assign(name, value, *exported);
            }
            StatementKind::Function(function, exported) => {
                if !scope.captured.replace(true) {
                    self.captured_scopes.push(scope.clone());
                }
                let callable = Callable {
                    name: function.name.clone(),
                    kind: CallableKind::Script {
                        function: function.clone(),
                        scope: Rc::downgrade(scope),
                    },
                };
                scope.assign(
                    &function.name,
                    Value::Function(Rc::new(callable)),
                    *exported,
                );
            }
            StatementKind::Import(import, name) => {
                let module = self.import(import, &scope.path)?;
                scope.assign(name, module, false);
            }
            StatementKind::If(branches, otherwise) => {
                for (condition, body) in branches {
                    if self.condition(condition, scope)? {
                        return self.block(body, scope);
                    }
                }
                return self.block(otherwise, scope);
            }
            StatementKind::While(condition, body) => {
                while self.condition(condition, scope)? {
                    match self.block(body, scope)? {
                        Flow::Next | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            StatementKind::For(name, items, body) => {
                for item in self.expression(items, scope)?.items()? {
                    scope.assign(name, item, false);
                    match self.block(body, scope)? {
                        Flow::Next | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            StatementKind::Try(body, name, handler) => {
                return match self.block(body, scope) {
                    Ok(flow) => Ok(flow),
                    Err(error) => {
                        scope.assign(name, error.into_value(), false);
                        self.block(handler, scope)
                    }
                };
            }
            StatementKind::Return(value) => {
                let value = match value {
                    Some(expression) => self.expression(expression, scope)?,
                    None => Value::Null,
                };
                return Ok(Flow::Return(value));
            }
            StatementKind::Break => return Ok(Flow::Break),
            StatementKind::Continue => return Ok(Flow::Continue),
            StatementKind::Pass => {}
            StatementKind::Expression(expression) => {
                self.expression(expression, scope)?;
            }
        }
        Ok(Flow::Next)
    }

    fn condition(&mut self, condition: &Expression, scope: &Rc<Scope>) -> Result<bool> {
        self.expression(condition, scope)?
            .as_bool()
            .map_err(|error| error.at(&scope.path, condition.at))
    }

    fn expression(&mut self, expression: &Expression, scope: &Rc<Scope>) -> Result<Value> {
        self.expression_inner(expression, scope)
            .map_err(|error| error.at(&scope.path, expression.at))
    }

    fn expression_inner(&mut self, expression: &Expression, scope: &Rc<Scope>) -> Result<Value> {
        match &expression.kind {
            ExpressionKind::Literal(value) => Ok(value.clone()),
            ExpressionKind::Name(name) => scope.get(name),
            ExpressionKind::List(items) => {
                let values = items
                    .iter()
                    .map(|item| self.expression(item, scope))
                    .collect::<Result<_>>()?;
                Ok(Value::list(values))
            }
            ExpressionKind::Map(items) => {
                let values = items
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), self.expression(value, scope)?)))
                    .collect::<Result<_>>()?;
                Ok(Value::map(values))
            }
            ExpressionKind::Field(object, name) => self.expression(object, scope)?.field(name),
            ExpressionKind::Index(object, index) => self
                .expression(object, scope)?
                .index(&self.expression(index, scope)?),
            ExpressionKind::Unary(operator, operand) => {
                let value = self.expression(operand, scope)?;
                match (operator, value) {
                    (Unary::Not, value) => Ok(Value::Bool(!value.as_bool()?)),
                    (Unary::Negate, Value::Int(value)) => checked_int(value.checked_neg()),
                    (Unary::Negate, Value::Float(value)) => Ok(Value::Float(-value)),
                    (Unary::Negate, value) => Err(value.expected("a number")),
                }
            }
            ExpressionKind::Binary(left, operator, right) => {
                let left = self.expression(left, scope)?;
                if matches!(operator, Binary::And | Binary::Or) {
                    let left = left.as_bool()?;
                    if left == matches!(operator, Binary::Or) {
                        return Ok(Value::Bool(left));
                    }
                    return Ok(Value::Bool(self.condition(right, scope)?));
                }
                let right = self.expression(right, scope)?;
                binary(left, *operator, right)
            }
            ExpressionKind::Call(callee, arguments) => {
                let callee = if let ExpressionKind::Field(object, field) = &callee.kind {
                    let receiver = self.expression(object, scope)?;
                    if field == "append"
                        && let Value::List(list) = receiver
                    {
                        check_arity("list.append", 1, arguments.len())?;
                        list.append(self.expression(&arguments[0], scope)?)?;
                        return Ok(Value::Null);
                    }
                    receiver.field(field)?
                } else {
                    self.expression(callee, scope)?
                };
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.expression(argument, scope)?);
                }
                self.call(&callee, values)
            }
        }
    }

    fn call(&mut self, callee: &Value, values: Vec<Value>) -> Result<Value> {
        let Value::Function(callable) = callee else {
            return Err(callee.expected("a function"));
        };
        match &callable.kind {
            CallableKind::Native { arity, call } => {
                if let Some(arity) = arity {
                    check_arity(&callable.name, *arity, values.len())?;
                }
                call(&values)
            }
            CallableKind::Script { function, scope } => {
                check_arity(&callable.name, function.parameters.len(), values.len())?;
                let parent = scope.upgrade().ok_or_else(|| {
                    Error::new("ValueError", "function belongs to a finished execution")
                })?;
                if self.call_depth >= 64 {
                    return Err(Error::new("RecursionError", "call depth exceeds 64 levels"));
                }
                let mut local_names = BTreeSet::new();
                collect_locals(&function.body, &mut local_names);
                let scope = Rc::new(Scope {
                    path: parent.path.clone(),
                    names: RefCell::new(function.parameters.iter().cloned().zip(values).collect()),
                    exports: RefCell::default(),
                    local_names,
                    parent: Some(parent),
                    captured: Cell::new(false),
                });
                self.call_depth += 1;
                let result = self.block(&function.body, &scope);
                self.call_depth -= 1;
                match result? {
                    Flow::Return(value) => Ok(value),
                    _ => Ok(Value::Null),
                }
            }
        }
    }

    fn import(&mut self, import: &Import, source: &Path) -> Result<Value> {
        let requested = match import {
            Import::Standard(name) => {
                return self.config.modules.get(name).cloned().ok_or_else(|| {
                    Error::new("ImportError", format!("module '{name}' is unavailable"))
                });
            }
            Import::File(path) => path,
        };
        if !self.config.file_imports {
            return Err(Error::new("ImportError", "file imports are unavailable"));
        }
        let path = source.parent().unwrap_or(Path::new("")).join(requested);
        let canonical = fs::canonicalize(&path).map_err(|error| file_error(&path, error))?;
        if let Some(module) = self.modules.get(&canonical) {
            return Ok(module.clone());
        }
        if self.loading.len() >= 32 {
            return Err(Error::new(
                "ImportError",
                "module nesting exceeds 32 levels",
            ));
        }
        if !self.loading.insert(canonical.clone()) {
            return Err(Error::new(
                "ImportError",
                format!("cyclic import of '{}'", path.display()),
            ));
        }
        let result = self.load_module(&canonical);
        self.loading.remove(&canonical);
        let module = result?;
        self.modules.insert(canonical, module.clone());
        Ok(module)
    }

    fn load_module(&mut self, path: &Path) -> Result<Value> {
        let source = fs::read_to_string(path).map_err(|error| file_error(path, error))?;
        let program = parser::parse(&source, path)?;
        let scope = Scope::file(path, &self.config.globals);
        self.block(&program, &scope)?;
        Ok(scope.module())
    }
}

fn collect_locals(statements: &Block, locals: &mut BTreeSet<String>) {
    for statement in statements {
        match &statement.kind {
            StatementKind::Assign(name, _, _) | StatementKind::Import(_, name) => {
                locals.insert(name.clone());
            }
            StatementKind::Function(function, _) => {
                locals.insert(function.name.clone());
            }
            StatementKind::If(branches, otherwise) => {
                for (_, body) in branches {
                    collect_locals(body, locals);
                }
                collect_locals(otherwise, locals);
            }
            StatementKind::For(name, _, body) => {
                locals.insert(name.clone());
                collect_locals(body, locals);
            }
            StatementKind::While(_, body) => collect_locals(body, locals),
            StatementKind::Try(body, name, handler) => {
                locals.insert(name.clone());
                collect_locals(body, locals);
                collect_locals(handler, locals);
            }
            _ => {}
        }
    }
}

pub(crate) fn file_error(path: &Path, error: std::io::Error) -> Error {
    Error::new("IoError", format!("{}: {error}", path.display()))
}

fn check_arity(name: &str, expected: usize, actual: usize) -> Result<()> {
    if expected != actual {
        return Err(Error::new(
            "TypeError",
            format!("{name} expects {expected} argument(s), got {actual}"),
        ));
    }
    Ok(())
}

fn checked_int(value: Option<i64>) -> Result<Value> {
    value
        .map(Value::Int)
        .ok_or_else(|| Error::new("OverflowError", "integer overflow"))
}

fn binary(left: Value, op: Binary, right: Value) -> Result<Value> {
    use Binary::*;
    match op {
        Equal => return Ok(Value::Bool(left == right)),
        NotEqual => return Ok(Value::Bool(left != right)),
        Less | LessEqual | Greater | GreaterEqual => {
            let order = left.compare(&right).ok_or_else(|| {
                Error::new(
                    "TypeError",
                    format!(
                        "cannot compare {} and {}",
                        left.type_name(),
                        right.type_name()
                    ),
                )
            })?;
            return Ok(Value::Bool(match op {
                Less => order == Ordering::Less,
                LessEqual => order != Ordering::Greater,
                Greater => order == Ordering::Greater,
                GreaterEqual => order != Ordering::Less,
                _ => unreachable!(),
            }));
        }
        Add => {
            if let (Value::String(a), Value::String(b)) = (&left, &right) {
                return Ok(Value::String(format!("{a}{b}")));
            }
        }
        _ => {}
    }
    if matches!(op, Divide | Remainder)
        && (matches!(right, Value::Int(0)) || matches!(right, Value::Float(value) if value == 0.0))
    {
        return Err(Error::new("ZeroDivisionError", "division by zero"));
    }
    if let (Value::Int(a), Value::Int(b)) = (&left, &right) {
        match op {
            Add => return checked_int(a.checked_add(*b)),
            Subtract => return checked_int(a.checked_sub(*b)),
            Multiply => return checked_int(a.checked_mul(*b)),
            Remainder => return checked_int(a.checked_rem(*b)),
            _ => {}
        }
    }
    let number = |value: Value| match value {
        Value::Float(value) => Ok(value),
        Value::Int(value) => Ok(value as f64),
        other => Err(other.expected("a number")),
    };
    let (left, right) = (number(left)?, number(right)?);
    let result = match op {
        Add => left + right,
        Subtract => left - right,
        Multiply => left * right,
        Divide => left / right,
        Remainder => left % right,
        _ => unreachable!(),
    };
    if result.is_finite() {
        Ok(Value::Float(result))
    } else {
        Err(Error::new(
            "OverflowError",
            "floating-point result is out of range",
        ))
    }
}
