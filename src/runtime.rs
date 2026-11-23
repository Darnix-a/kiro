use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, Weak};

use crate::command::{self, CmdResult};
use crate::environment::Environment;
use crate::error::{Error, Result};
use crate::lexer::{Part, Template};
use crate::parser::{
    self, Argument, Binary, Block, Expression, ExpressionKind, Function, Import, Statement,
    StatementKind, Unary,
};
use crate::{Value, standard};

mod builtins;
mod language;
use builtins::Builtin;
pub use language::{EnumValue, StructValue};
use language::{EvaluatedArgs, StructType};

type Values = BTreeMap<String, Value>;
type LocalFunction = dyn Fn(&[Value]) -> Result<Value>;
type NativeFunction = dyn Fn(&[Value]) -> Result<Value> + Send + Sync;
type LocalFunctions = BTreeMap<u64, Box<LocalFunction>>;

pub struct Callable {
    name: String,
    kind: CallableKind,
}

enum CallableKind {
    Builtin(Builtin),
    Local {
        arity: usize,
        id: u64,
    },
    Native {
        arity: Option<usize>,
        call: Box<NativeFunction>,
    },
    Script {
        function: Arc<Function>,
        scope: Weak<Scope>,
        receiver: Option<Value>,
    },
    Constructor(Arc<StructType>),
}

impl Callable {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn receiver(&self) -> Option<&Value> {
        match &self.kind {
            CallableKind::Script { receiver, .. } => receiver.as_ref(),
            _ => None,
        }
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
    call: impl Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
) -> Value {
    Value::Function(Arc::new(Callable {
        name: name.into(),
        kind: CallableKind::Native {
            arity,
            call: Box::new(call),
        },
    }))
}

/// Configures which capabilities a new script execution can access.
pub struct Interpreter {
    config: SharedConfig,
    locals: LocalFunctions,
}

#[derive(Default)]
struct SharedConfig {
    globals: Values,
    modules: Values,
    file_imports: bool,
    command_literals: bool,
    environment: Arc<Environment>,
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
            config: SharedConfig::default(),
            locals: LocalFunctions::new(),
        };
        interpreter.define_thread_safe_function("len", 1, |args| args[0].length());
        interpreter.define_thread_safe_function("lines", 1, |args| args[0].lines());
        interpreter.set_global("range", native("range", None, crate::general::range));
        for (name, kind) in Builtin::PRELUDE {
            interpreter.set_global(
                name,
                Value::Function(Arc::new(Callable {
                    name: name.into(),
                    kind: CallableKind::Builtin(kind),
                })),
            );
        }
        interpreter
    }

    /// Creates an interpreter with the standard command-line capabilities.
    pub fn cli() -> Self {
        let mut interpreter = Self::new();
        interpreter.set_global("print", standard::print_function());
        interpreter.set_global("input", standard::input_function());
        for name in standard::MODULES {
            interpreter
                .enable_standard_module(name)
                .expect("known standard module");
        }
        interpreter.allow_file_imports();
        interpreter
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.config.globals.insert(name.into(), value);
    }

    pub fn define_function(
        &mut self,
        name: impl Into<String>,
        arity: usize,
        call: impl Fn(&[Value]) -> Result<Value> + 'static,
    ) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, AtomicOrdering::Relaxed);
        self.locals.insert(id, Box::new(call));
        let name = name.into();
        self.set_global(
            name.clone(),
            Value::Function(Arc::new(Callable {
                name,
                kind: CallableKind::Local { id, arity },
            })),
        );
    }

    /// Exposes a host function that may also run in a parallel task.
    pub fn define_thread_safe_function(
        &mut self,
        name: impl Into<String>,
        arity: usize,
        call: impl Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    ) {
        let name = name.into();
        self.set_global(name.clone(), native(&name, Some(arity), call));
    }

    pub fn add_module(&mut self, name: impl Into<String>, exports: Values) {
        self.config.modules.insert(name.into(), Value::map(exports));
    }

    /// Preloads a named standard module and also makes it available through import.
    pub fn enable_standard_module(&mut self, name: &str) -> Result<()> {
        let module = standard::module(name, &self.config.environment).ok_or_else(|| {
            Error::new("ImportError", format!("unknown standard module '{name}'"))
        })?;
        self.config.modules.insert(name.into(), module.clone());
        self.set_global(name, module);
        if name == "process" {
            self.config.command_literals = true;
        }
        Ok(())
    }

    /// Allows scripts to load source files through relative or absolute imports.
    pub fn allow_file_imports(&mut self) {
        self.config.file_imports = true;
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
        let mut runtime = self.runtime(path);
        let scope = Scope::file(path, &self.config.globals);
        match runtime.block(&program, &scope)? {
            Flow::Return(value) => Ok(value),
            _ => Ok(Value::Null),
        }
    }

    pub fn repl(&self) -> Repl<'_> {
        let path = Path::new("<repl>");
        Repl {
            runtime: self.runtime(path),
            scope: Scope::file(path, &self.config.globals),
        }
    }

    fn runtime(&self, path: &Path) -> Runtime<'_> {
        Runtime {
            config: &self.config,
            locals: Some(&self.locals),
            modules: BTreeMap::new(),
            loading: BTreeSet::from([path.to_path_buf()]),
            captured_scopes: Vec::new(),
            call_depth: 0,
        }
    }
}

/// A sequence of submissions sharing one script scope and import cache.
pub struct Repl<'a> {
    runtime: Runtime<'a>,
    scope: Arc<Scope>,
}

impl Repl<'_> {
    pub fn needs_more_input(source: &str) -> bool {
        let lines = source.lines().count();
        let blank = source
            .lines()
            .last()
            .is_some_and(|line| line.trim().is_empty());
        match parser::parse(source, Path::new("<repl>")) {
            Err(error) => {
                error.message == "unclosed delimiter"
                    || (!blank
                        && matches!(
                            error.message.as_str(),
                            "expected an indented block" | "expected 'catch' after the try block"
                        )
                        && error.location.is_some_and(|at| at.line > lines))
            }
            Ok(program) => {
                !blank
                    && program.last().is_some_and(|statement| {
                        matches!(
                            statement.kind,
                            StatementKind::Function(..)
                                | StatementKind::Struct(..)
                                | StatementKind::Enum(..)
                                | StatementKind::Match(..)
                                | StatementKind::If(..)
                                | StatementKind::For(..)
                                | StatementKind::While(..)
                                | StatementKind::Try(..)
                        )
                    })
            }
        }
    }

    pub fn eval(&mut self, source: &str) -> Result<Value> {
        let program = parser::parse(source, &self.scope.path)?;
        if let Some((last, preceding)) = program.split_last()
            && let StatementKind::Expression(expression) = &last.kind
        {
            if let Flow::Return(value) = self.runtime.block(preceding, &self.scope)? {
                return Ok(value);
            }
            return self.runtime.expression(expression, &self.scope);
        }
        match self.runtime.block(&program, &self.scope)? {
            Flow::Return(value) => Ok(value),
            _ => Ok(Value::Null),
        }
    }
}

pub(crate) fn http_serve_function() -> Value {
    Value::Function(Arc::new(Callable {
        name: "http.serve".into(),
        kind: CallableKind::Builtin(Builtin::Serve),
    }))
}

struct Scope {
    path: Arc<PathBuf>,
    names: Mutex<Values>,
    exports: Mutex<BTreeSet<String>>,
    local_names: BTreeSet<String>,
    parent: Option<Arc<Scope>>,
    captured: AtomicBool,
}

impl Scope {
    fn file(path: &Path, globals: &Values) -> Arc<Self> {
        Arc::new(Self {
            path: Arc::new(path.to_path_buf()),
            names: Mutex::new(globals.clone()),
            exports: Mutex::default(),
            local_names: BTreeSet::new(),
            parent: None,
            captured: AtomicBool::new(false),
        })
    }

    fn get(&self, name: &str) -> Result<Value> {
        if let Some(value) = self.names.lock().unwrap().get(name) {
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
        self.names.lock().unwrap().insert(name.to_owned(), value);
        if exported {
            self.exports.lock().unwrap().insert(name.to_owned());
        }
    }

    fn module(&self) -> Value {
        let names = self.names.lock().unwrap();
        Value::map(
            self.exports
                .lock()
                .unwrap()
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
    config: &'a SharedConfig,
    locals: Option<&'a LocalFunctions>,
    modules: BTreeMap<PathBuf, Value>,
    loading: BTreeSet<PathBuf>,
    captured_scopes: Vec<Arc<Scope>>,
    call_depth: usize,
}

impl Runtime<'_> {
    fn block(&mut self, statements: &[Statement], scope: &Arc<Scope>) -> Result<Flow> {
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

    fn statement(&mut self, statement: &Statement, scope: &Arc<Scope>) -> Result<Flow> {
        match &statement.kind {
            StatementKind::Assign(name, expression, exported) => {
                let value = self.expression(expression, scope)?;
                scope.assign(name, value, *exported);
            }
            StatementKind::Function(function, exported) => {
                let callable = self.script_function(function, scope);
                scope.assign(&function.name, callable, *exported);
            }
            StatementKind::Struct(declaration, exported) => {
                let constructor = self.struct_constructor(declaration, scope);
                scope.assign(&declaration.name, constructor, *exported);
            }
            StatementKind::Enum(name, members, exported) => {
                let tag = Arc::new(name.clone());
                let entries = members
                    .iter()
                    .map(|member| {
                        (
                            member.clone(),
                            Value::Enum(Arc::new(EnumValue::new(tag.clone(), member.clone()))),
                        )
                    })
                    .collect();
                scope.assign(name, Value::map(entries), *exported);
            }
            StatementKind::Destructure(pattern, expression) => {
                let subject = self.expression(expression, scope)?;
                let mut bindings = Values::new();
                if !self.match_pattern(pattern, &subject, scope, &mut bindings)? {
                    return Err(Error::new(
                        "PatternError",
                        "value does not match the assignment pattern",
                    ));
                }
                scope.names.lock().unwrap().extend(bindings);
            }
            StatementKind::Match(expression, cases) => {
                let subject = self.expression(expression, scope)?;
                return self.match_cases(&subject, cases, scope);
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
                    Err(error) if error.exit_code().is_some() => Err(error),
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
            StatementKind::Command(template) => {
                let command = self.command_args(template, scope)?;
                let args: Vec<_> = command[1..].iter().map(String::as_str).collect();
                command::stream(&command[0], &args, &self.config.environment)?;
            }
            StatementKind::Expression(expression) => {
                self.expression(expression, scope)?;
            }
        }
        Ok(Flow::Next)
    }

    fn condition(&mut self, condition: &Expression, scope: &Arc<Scope>) -> Result<bool> {
        self.expression(condition, scope)?
            .as_bool()
            .map_err(|error| error.at(&scope.path, condition.at))
    }

    fn expression(&mut self, expression: &Expression, scope: &Arc<Scope>) -> Result<Value> {
        self.expression_inner(expression, scope)
            .map_err(|error| error.at(&scope.path, expression.at))
    }

    fn expression_inner(&mut self, expression: &Expression, scope: &Arc<Scope>) -> Result<Value> {
        match &expression.kind {
            ExpressionKind::Literal(value) => Ok(value.clone()),
            ExpressionKind::Interpolated(parts) => {
                Ok(self.interpolate(parts, scope, false)?.into())
            }
            ExpressionKind::Command(template) => {
                let command = self.command_args(template, scope)?;
                let args: Vec<_> = command[1..].iter().map(String::as_str).collect();
                let output = command::execute(&command[0], &args, &self.config.environment)?;
                Ok(Value::Command(Arc::new(CmdResult::from_output(output))))
            }
            ExpressionKind::Lambda(function) => Ok(self.script_function(function, scope)),
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
            ExpressionKind::Field(object, name, safe) => {
                let receiver = self.expression(object, scope)?;
                receiver.member(name, *safe)
            }
            ExpressionKind::Index(object, index) => self
                .expression(object, scope)?
                .index(&self.expression(index, scope)?),
            ExpressionKind::Slice(object, start, end) => {
                let value = self.expression(object, scope)?;
                let start = self.slice_bound(start.as_deref(), scope)?;
                let end = self.slice_bound(end.as_deref(), scope)?;
                value.slice(start, end)
            }
            ExpressionKind::ListComp(value, clause) => {
                let output = self.comprehend(clause, scope, |runtime, scope| {
                    runtime.expression(value, scope)
                })?;
                Ok(Value::list(output))
            }
            ExpressionKind::MapComp(key, value, clause) => {
                let output = self.comprehend(clause, scope, |runtime, scope| {
                    let key = runtime.expression(key, scope)?.as_str()?.to_owned();
                    Ok((key, runtime.expression(value, scope)?))
                })?;
                Ok(Value::map(output.into_iter().collect()))
            }
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
                if matches!(operator, Binary::Coalesce) {
                    return if matches!(left, Value::Null) {
                        self.expression(right, scope)
                    } else {
                        Ok(left)
                    };
                }
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
                self.call_expression(callee, arguments, scope, None)
            }
            ExpressionKind::Pipe(input, callee, arguments) => {
                let input = self.expression(input, scope)?;
                self.call_expression(callee, arguments, scope, Some(input))
            }
        }
    }

    fn script_function(&mut self, function: &Arc<Function>, scope: &Arc<Scope>) -> Value {
        self.retain_scope(scope);
        Value::Function(Arc::new(Callable {
            name: function.name.clone(),
            kind: CallableKind::Script {
                function: function.clone(),
                scope: Arc::downgrade(scope),
                receiver: None,
            },
        }))
    }

    fn retain_scope(&mut self, scope: &Arc<Scope>) {
        if !scope.captured.swap(true, AtomicOrdering::Relaxed) {
            self.captured_scopes.push(scope.clone());
        }
    }

    fn command_args(
        &mut self,
        template: &Template<Expression>,
        scope: &Arc<Scope>,
    ) -> Result<Vec<String>> {
        if !self.config.command_literals {
            return Err(Error::new(
                "ProcessError",
                "command literals require the standard process module",
            ));
        }
        template
            .iter()
            .map(|parts| self.interpolate(parts, scope, true))
            .collect()
    }

    fn interpolate(
        &mut self,
        parts: &[Part<Expression>],
        scope: &Arc<Scope>,
        command: bool,
    ) -> Result<String> {
        let mut text = String::new();
        for part in parts {
            match part {
                Part::Text(literal) => text.push_str(literal),
                Part::Expression(expression) => {
                    let value = self.expression(expression, scope)?;
                    if command
                        && !matches!(
                            value,
                            Value::String(_) | Value::Bool(_) | Value::Int(_) | Value::Float(_)
                        )
                    {
                        return Err(value
                            .expected("a string, boolean, or number for a command argument")
                            .at(&scope.path, expression.at));
                    }
                    text.push_str(&value.to_string());
                }
            }
        }
        Ok(text)
    }

    fn call_expression(
        &mut self,
        callee: &Expression,
        arguments: &[Argument],
        scope: &Arc<Scope>,
        piped: Option<Value>,
    ) -> Result<Value> {
        let arity = arguments.len() + usize::from(piped.is_some());
        let optional = matches!(callee.kind, ExpressionKind::Field(_, _, true));
        let callee = if let ExpressionKind::Field(object, field, safe) = &callee.kind {
            let receiver = self.expression(object, scope)?;
            if *safe && matches!(receiver, Value::Null) {
                return Ok(Value::Null);
            }
            match (field.as_str(), &receiver) {
                ("append", Value::List(list)) => {
                    check_arity("list.append", 1, arity)?;
                    let item = self
                        .evaluate_args(arguments, scope, piped)?
                        .positional_only("list.append")?
                        .pop()
                        .unwrap();
                    list.append(item)?;
                    return Ok(Value::Null);
                }
                ("lines", Value::String(_) | Value::Command(_)) => {
                    check_arity(".lines", 0, arity)?;
                    return receiver.lines();
                }
                (method, Value::String(text)) => {
                    if let Some((count, call)) = standard::string_method(method) {
                        check_arity(method, count, arity)?;
                        let values = self
                            .evaluate_args(arguments, scope, piped)?
                            .positional_only(method)?;
                        return call(text, &values);
                    }
                    receiver.member(field, *safe)?
                }
                ("json", Value::Map(fields))
                    if !fields.contains_key("json") && fields.contains_key("body") =>
                {
                    check_arity(".json", 0, arity)?;
                    return standard::parse_json(fields["body"].as_str()?);
                }
                _ => receiver.member(field, *safe)?,
            }
        } else {
            self.expression(callee, scope)?
        };
        if optional && matches!(callee, Value::Null) {
            return Ok(Value::Null);
        }
        let values = self.evaluate_args(arguments, scope, piped)?;
        self.call_with(&callee, values)
    }

    fn call(&mut self, callee: &Value, values: Vec<Value>) -> Result<Value> {
        self.call_with(callee, EvaluatedArgs::positional(values))
    }

    fn call_with(&mut self, callee: &Value, values: EvaluatedArgs) -> Result<Value> {
        let Value::Function(callable) = callee else {
            return Err(callee.expected("a function"));
        };
        match &callable.kind {
            CallableKind::Builtin(kind) => {
                self.builtin(*kind, values.positional_only(&callable.name)?)
            }
            CallableKind::Local { id, arity } => {
                let values = values.positional_only(&callable.name)?;
                check_arity(&callable.name, *arity, values.len())?;
                let call = self
                    .locals
                    .and_then(|functions| functions.get(id))
                    .ok_or_else(|| {
                        Error::new(
                            "ValueError",
                            format!(
                                "host function '{}' is unavailable in this execution",
                                callable.name
                            ),
                        )
                    })?;
                call(&values)
            }
            CallableKind::Native { arity, call } => {
                let values = values.positional_only(&callable.name)?;
                if let Some(arity) = arity {
                    check_arity(&callable.name, *arity, values.len())?;
                }
                call(&values)
            }
            CallableKind::Script {
                function,
                scope,
                receiver,
            } => self.call_script(function, scope, receiver, values),
            CallableKind::Constructor(structure) => self.construct(structure, values),
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
            StatementKind::Struct(declaration, _) => {
                locals.insert(declaration.name.clone());
            }
            StatementKind::Enum(name, _, _) => {
                locals.insert(name.clone());
            }
            StatementKind::Destructure(pattern, _) => pattern.names(locals),
            StatementKind::Match(_, cases) => {
                for case in cases {
                    case.pattern.names(locals);
                    collect_locals(&case.body, locals);
                }
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
        In => return right.contains(&left),
        Contains => return left.contains(&right),
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
