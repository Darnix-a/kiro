use super::*;
use crate::parser::{Case, Comprehension, Parameter, Pattern, StructDecl};

#[derive(Debug)]
pub struct EnumValue {
    name: Arc<String>,
    member: String,
}

impl EnumValue {
    pub(super) fn new(name: Arc<String>, member: String) -> Self {
        Self { name, member }
    }

    pub fn enum_name(&self) -> &str {
        &self.name
    }

    pub fn member(&self) -> &str {
        &self.member
    }
}

impl PartialEq for EnumValue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.name, &other.name) && self.member == other.member
    }
}

#[derive(Debug)]
pub(super) struct StructType {
    declaration: Arc<StructDecl>,
    scope: Weak<Scope>,
}

#[derive(Debug)]
pub struct StructValue {
    structure: Arc<StructType>,
    pub(crate) fields: Values,
}

impl StructValue {
    pub fn name(&self) -> &str {
        &self.structure.declaration.name
    }

    pub fn fields(&self) -> &BTreeMap<String, Value> {
        &self.fields
    }

    pub(crate) fn field(self: &Arc<Self>, name: &str) -> Result<Value> {
        if let Some(value) = self.fields.get(name) {
            return Ok(value.clone());
        }
        let method = self
            .structure
            .declaration
            .methods
            .iter()
            .find(|method| method.name == name)
            .ok_or_else(|| {
                Error::new(
                    "KeyError",
                    format!("{} has no member named '{name}'", self.name()),
                )
            })?;
        Ok(Value::Function(Arc::new(Callable {
            name: format!("{}.{}", self.name(), method.name),
            kind: CallableKind::Script {
                function: method.clone(),
                scope: self.structure.scope.clone(),
                receiver: Some(Value::Struct(self.clone())),
            },
        })))
    }
}

impl PartialEq for StructValue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.structure, &other.structure) && self.fields == other.fields
    }
}

pub(super) struct EvaluatedArgs {
    positional: Vec<Value>,
    named: Values,
}

impl EvaluatedArgs {
    pub(super) fn positional(positional: Vec<Value>) -> Self {
        Self {
            positional,
            named: Values::new(),
        }
    }

    pub(super) fn positional_only(self, name: &str) -> Result<Vec<Value>> {
        if !self.named.is_empty() {
            return Err(Error::new(
                "TypeError",
                format!("{name} takes positional arguments"),
            ));
        }
        Ok(self.positional)
    }
}

impl Scope {
    fn child(parent: &Arc<Self>, names: Values, local_names: BTreeSet<String>) -> Arc<Self> {
        Arc::new(Self {
            path: parent.path.clone(),
            names: Mutex::new(names),
            exports: Mutex::default(),
            local_names,
            parent: Some(parent.clone()),
            captured: AtomicBool::new(false),
        })
    }
}

impl Runtime<'_> {
    pub(super) fn evaluate_args(
        &mut self,
        arguments: &[Argument],
        scope: &Arc<Scope>,
        piped: Option<Value>,
    ) -> Result<EvaluatedArgs> {
        let mut args = EvaluatedArgs::positional(piped.into_iter().collect());
        for argument in arguments {
            if let Some(name) = &argument.name {
                if args.named.contains_key(name) {
                    return Err(Error::new(
                        "TypeError",
                        format!("duplicate keyword argument '{name}'"),
                    ));
                }
                args.named
                    .insert(name.clone(), self.expression(&argument.value, scope)?);
            } else {
                if !args.named.is_empty() {
                    return Err(Error::new(
                        "TypeError",
                        "positional arguments must precede keyword arguments",
                    ));
                }
                args.positional
                    .push(self.expression(&argument.value, scope)?);
            }
        }
        Ok(args)
    }

    pub(super) fn call_script(
        &mut self,
        function: &Function,
        parent: &Weak<Scope>,
        receiver: &Option<Value>,
        mut args: EvaluatedArgs,
    ) -> Result<Value> {
        let parent = defining_scope(parent)?;
        if let Some(receiver) = receiver {
            args.positional.insert(0, receiver.clone());
        }
        let mut locals = BTreeSet::new();
        collect_locals(&function.body, &mut locals);
        locals.extend(function.parameters.iter().map(|param| param.name.clone()));
        let scope = Scope::child(&parent, Values::new(), locals);
        self.nested_call(|runtime| {
            runtime.bind_parameters(&function.name, &function.parameters, args, &scope)?;
            match runtime.block(&function.body, &scope)? {
                Flow::Return(value) => Ok(value),
                _ => Ok(Value::Null),
            }
        })
    }

    pub(super) fn struct_constructor(
        &mut self,
        declaration: &Arc<StructDecl>,
        scope: &Arc<Scope>,
    ) -> Value {
        self.retain_scope(scope);
        Value::Function(Arc::new(Callable {
            name: declaration.name.clone(),
            kind: CallableKind::Constructor(Arc::new(StructType {
                declaration: declaration.clone(),
                scope: Arc::downgrade(scope),
            })),
        }))
    }

    pub(super) fn construct(
        &mut self,
        structure: &Arc<StructType>,
        args: EvaluatedArgs,
    ) -> Result<Value> {
        let parent = defining_scope(&structure.scope)?;
        let decl = &structure.declaration;
        let scope = Scope::child(
            &parent,
            Values::new(),
            decl.fields.iter().map(|field| field.name.clone()).collect(),
        );
        self.nested_call(|runtime| {
            runtime.bind_parameters(&decl.name, &decl.fields, args, &scope)?;
            let fields = scope.names.lock().unwrap().clone();
            Ok(Value::Struct(Arc::new(StructValue {
                structure: structure.clone(),
                fields,
            })))
        })
    }

    fn bind_parameters(
        &mut self,
        name: &str,
        params: &[Parameter],
        args: EvaluatedArgs,
        scope: &Arc<Scope>,
    ) -> Result<()> {
        let EvaluatedArgs {
            positional,
            mut named,
        } = args;
        if positional.len() > params.len() {
            return Err(Error::new(
                "TypeError",
                format!("{name} takes at most {} arguments", params.len()),
            ));
        }
        let mut supplied = positional.into_iter().map(Some).collect::<Vec<_>>();
        supplied.resize(params.len(), None);
        for (parameter, slot) in params.iter().zip(&mut supplied) {
            if let Some(value) = named.remove(&parameter.name) {
                if slot.is_some() {
                    return Err(Error::new(
                        "TypeError",
                        format!("{name} got multiple values for '{}'", parameter.name),
                    ));
                }
                *slot = Some(value);
            }
            if slot.is_none() && parameter.default.is_none() {
                return Err(Error::new(
                    "TypeError",
                    format!("{name} is missing '{}'", parameter.name),
                ));
            }
        }
        if let Some((unknown, _)) = named.first_key_value() {
            return Err(Error::new(
                "TypeError",
                format!("{name} has no parameter named '{unknown}'"),
            ));
        }
        for (parameter, slot) in params.iter().zip(supplied) {
            let value = match slot {
                Some(value) => value,
                None => self.expression(parameter.default.as_ref().unwrap(), scope)?,
            };
            if let Some(annotation) = &parameter.annotation
                && !matches_type(&value, annotation, scope)?
            {
                return Err(Error::new(
                    "TypeError",
                    format!(
                        "{} expects {annotation}, got {}",
                        parameter.name,
                        value.type_name()
                    ),
                ));
            }
            scope.assign(&parameter.name, value, false);
        }
        Ok(())
    }

    fn nested_call(&mut self, run: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        if self.call_depth >= 64 {
            return Err(Error::new("RecursionError", "call depth exceeds 64 levels"));
        }
        self.call_depth += 1;
        let result = run(self);
        self.call_depth -= 1;
        result
    }

    pub(super) fn match_cases(
        &mut self,
        value: &Value,
        cases: &[Case],
        scope: &Arc<Scope>,
    ) -> Result<Flow> {
        for case in cases {
            let mut bindings = Values::new();
            if !self.match_pattern(&case.pattern, value, scope, &mut bindings)? {
                continue;
            }
            if let Some(guard) = &case.guard {
                let guard_scope = Scope::child(scope, bindings.clone(), BTreeSet::new());
                if !self.condition(guard, &guard_scope)? {
                    continue;
                }
            }
            scope.names.lock().unwrap().extend(bindings);
            return self.block(&case.body, scope);
        }
        Ok(Flow::Next)
    }

    pub(super) fn match_pattern(
        &mut self,
        pattern: &Pattern,
        value: &Value,
        scope: &Arc<Scope>,
        bindings: &mut Values,
    ) -> Result<bool> {
        match pattern {
            Pattern::Wildcard => Ok(true),
            Pattern::Bind(name) => Ok(bind(bindings, name, value)),
            Pattern::Constant(constant) => Ok(self.expression(constant, scope)? == *value),
            Pattern::List(patterns, rest) => {
                let Value::List(list) = value else {
                    return Ok(false);
                };
                let values = list.values();
                if values.len() < patterns.len()
                    || (rest.is_none() && values.len() != patterns.len())
                {
                    return Ok(false);
                }
                for (pattern, value) in patterns.iter().zip(&values) {
                    if !self.match_pattern(pattern, value, scope, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(rest.as_ref().is_none_or(|name| {
                    name == "_"
                        || bind(
                            bindings,
                            name,
                            &Value::list(values[patterns.len()..].to_vec()),
                        )
                }))
            }
            Pattern::Map(patterns) => {
                let values = match value {
                    Value::Map(values) => values.as_ref(),
                    Value::Struct(instance) => &instance.fields,
                    _ => return Ok(false),
                };
                for (key, pattern) in patterns {
                    let Some(value) = values.get(key) else {
                        return Ok(false);
                    };
                    if !self.match_pattern(pattern, value, scope, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::Or(alternatives) => {
                for candidate in alternatives {
                    let mut local = bindings.clone();
                    if self.match_pattern(candidate, value, scope, &mut local)? {
                        *bindings = local;
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    pub(super) fn comprehend<T>(
        &mut self,
        clause: &Comprehension,
        scope: &Arc<Scope>,
        mut evaluate: impl FnMut(&mut Self, &Arc<Scope>) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut results = Vec::new();
        for item in self.expression(&clause.items, scope)?.items()? {
            let scope = Scope::child(
                scope,
                Values::from([(clause.name.clone(), item)]),
                BTreeSet::new(),
            );
            if let Some(guard) = &clause.guard
                && !self.condition(guard, &scope)?
            {
                continue;
            }
            results.push(evaluate(self, &scope)?);
        }
        Ok(results)
    }

    pub(super) fn slice_bound(
        &mut self,
        value: Option<&Expression>,
        scope: &Arc<Scope>,
    ) -> Result<Option<i64>> {
        value
            .map(|value| self.expression(value, scope)?.as_int())
            .transpose()
    }
}

fn defining_scope(scope: &Weak<Scope>) -> Result<Arc<Scope>> {
    scope
        .upgrade()
        .ok_or_else(|| Error::new("ValueError", "function belongs to a finished execution"))
}

fn bind(bindings: &mut Values, name: &str, value: &Value) -> bool {
    if let Some(bound) = bindings.get(name) {
        return bound == value;
    }
    bindings.insert(name.to_owned(), value.clone());
    true
}

fn matches_type(value: &Value, name: &str, scope: &Scope) -> Result<bool> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "any" => true,
        "int" | "integer" => matches!(value, Value::Int(_)),
        "float" => matches!(value, Value::Float(_)),
        "number" => matches!(value, Value::Int(_) | Value::Float(_)),
        "str" | "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "list" => matches!(value, Value::List(_)),
        "map" => matches!(value, Value::Map(_)),
        "function" => matches!(value, Value::Function(_)),
        _ => {
            let definition = scope
                .parent
                .as_ref()
                .unwrap()
                .get(name)
                .map_err(|_| Error::new("TypeError", format!("unknown type '{name}'")))?;
            match definition {
                Value::Function(constructor) => matches!(
                    (&constructor.kind, value),
                    (CallableKind::Constructor(structure), Value::Struct(instance)) if Arc::ptr_eq(structure, &instance.structure)
                ),
                Value::Map(values) => matches!(
                    value,
                    Value::Enum(member) if values.get(member.member()) == Some(value)
                ),
                _ => false,
            }
        }
    })
}
