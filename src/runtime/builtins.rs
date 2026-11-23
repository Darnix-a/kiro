use std::sync::atomic::AtomicUsize;
use std::thread;

use super::*;

#[derive(Clone, Copy)]
pub(super) enum Builtin {
    Filter,
    Map,
    Reduce,
    Any,
    All,
    Sort,
    Unique,
    Parallel,
    Retry,
    Exit,
    Serve,
}

impl Builtin {
    pub(super) const PRELUDE: [(&str, Self); 10] = [
        ("filter", Self::Filter),
        ("map", Self::Map),
        ("reduce", Self::Reduce),
        ("any", Self::Any),
        ("all", Self::All),
        ("sort", Self::Sort),
        ("unique", Self::Unique),
        ("parallel", Self::Parallel),
        ("retry", Self::Retry),
        ("exit", Self::Exit),
    ];
}

impl Runtime<'_> {
    pub(super) fn builtin(&mut self, kind: Builtin, values: Vec<Value>) -> Result<Value> {
        use Builtin::*;
        match kind {
            Filter | Map | Any | All => {
                let name = match kind {
                    Filter => "filter",
                    Map => "map",
                    Any => "any",
                    _ => "all",
                };
                check_arity(name, 2, values.len())?;
                check_callback(&values[1], 1)?;
                let mut output = Vec::new();
                for item in values[0].items()? {
                    let result = self.call(&values[1], vec![item.clone()])?;
                    match kind {
                        Map => output.push(result),
                        Filter if result.as_bool()? => output.push(item),
                        Any if result.as_bool()? => return Ok(Value::Bool(true)),
                        All if !result.as_bool()? => return Ok(Value::Bool(false)),
                        _ => {}
                    }
                }
                match kind {
                    Any => Ok(Value::Bool(false)),
                    All => Ok(Value::Bool(true)),
                    _ => Ok(Value::list(output)),
                }
            }
            Reduce => {
                check_arity("reduce", 3, values.len())?;
                check_callback(&values[2], 2)?;
                let mut accumulator = values[1].clone();
                for item in values[0].items()? {
                    accumulator = self.call(&values[2], vec![accumulator, item])?;
                }
                Ok(accumulator)
            }
            Sort => self.sort(&values),
            Unique => {
                check_arity("unique", 1, values.len())?;
                let mut output = Vec::new();
                for item in values[0].items()? {
                    if !output.contains(&item) {
                        output.push(item);
                    }
                }
                Ok(Value::list(output))
            }
            Parallel => {
                check_arity("parallel", 1, values.len())?;
                let Value::List(tasks) = &values[0] else {
                    return Err(values[0].expected("a list of zero-argument functions"));
                };
                self.parallel(&tasks.values())
            }
            Retry => {
                check_arity("retry", 3, values.len())?;
                let attempts = values[0].as_int()?;
                if attempts <= 0 {
                    return Err(Error::new("ValueError", "retry attempts must be positive"));
                }
                let delay = crate::time::milliseconds(&values[1])?;
                check_callback(&values[2], 0)?;
                let mut remaining = attempts;
                loop {
                    match self.call(&values[2], Vec::new()) {
                        Err(error) if error.exit_code().is_none() && remaining > 1 => {
                            remaining -= 1;
                            thread::sleep(delay);
                        }
                        result => return result,
                    }
                }
            }
            Exit => {
                let code = match values.as_slice() {
                    [] => 0,
                    [code] => u8::try_from(code.as_int()?).map_err(|_| {
                        Error::new("ValueError", "exit status must be between 0 and 255")
                    })?,
                    _ => return Err(Error::new("TypeError", "exit expects 0 or 1 arguments")),
                };
                Err(Error::exit(code))
            }
            Serve => {
                check_arity("http.serve", 2, values.len())?;
                check_callback(&values[1], 1)?;
                crate::http_server::serve(&values[0], |request| {
                    self.call(&values[1], vec![request])
                })
            }
        }
    }

    fn sort(&mut self, values: &[Value]) -> Result<Value> {
        let (items, callback) = match values {
            [items] => (items, None),
            [items, callback] => {
                check_callback(callback, 1)?;
                (items, Some(callback))
            }
            _ => return Err(Error::new("TypeError", "sort expects 1 or 2 arguments")),
        };
        let mut keyed = Vec::new();
        for item in items.items()? {
            let key = match callback {
                Some(callback) => self.call(callback, vec![item.clone()])?,
                None => item.clone(),
            };
            keyed.push((key, item));
        }
        if let Some((first, _)) = keyed.first()
            && keyed.iter().any(|(key, _)| first.compare(key).is_none())
        {
            return Err(Error::new(
                "TypeError",
                "sort keys must all be numbers or all be strings",
            ));
        }
        keyed.sort_by(|(left, _), (right, _)| left.compare(right).unwrap());
        Ok(Value::list(
            keyed.into_iter().map(|(_, item)| item).collect(),
        ))
    }

    fn parallel(&mut self, tasks: &[Value]) -> Result<Value> {
        for task in tasks {
            check_callback(task, 0)?;
            if matches!(task, Value::Function(function) if matches!(function.kind, CallableKind::Local { .. }))
            {
                return Err(Error::new(
                    "ValueError",
                    "parallel tasks require thread-safe host functions",
                ));
            }
        }
        if tasks.is_empty() {
            return Ok(Value::list(Vec::new()));
        }

        let workers = thread::available_parallelism()
            .map_or(2, |count| count.get())
            .clamp(2, 16);
        let next = AtomicUsize::new(0);
        let config = self.config;
        let depth = self.call_depth;
        let modules = &self.modules;
        let loading = &self.loading;

        let mut results = thread::scope(|scope| -> Result<_> {
            let mut handles = Vec::new();
            for _ in 0..tasks.len().min(workers) {
                let handle = thread::Builder::new()
                    .spawn_scoped(scope, || {
                        let mut runtime = Runtime {
                            config,
                            locals: None,
                            modules: modules.clone(),
                            loading: loading.clone(),
                            captured_scopes: Vec::new(),
                            call_depth: depth,
                        };
                        let mut results = Vec::new();
                        loop {
                            let index = next.fetch_add(1, AtomicOrdering::Relaxed);
                            let Some(task) = tasks.get(index) else { break };
                            results.push((index, runtime.call(task, Vec::new())));
                        }
                        (results, runtime.captured_scopes)
                    })
                    .map_err(|error| Error::new("ThreadError", error.to_string()))?;
                handles.push(handle);
            }

            let mut results = Vec::with_capacity(tasks.len());
            let mut panicked = false;
            for handle in handles {
                match handle.join() {
                    Ok((worker_results, scopes)) => {
                        results.extend(worker_results);
                        self.captured_scopes.extend(scopes);
                    }
                    Err(_) => panicked = true,
                }
            }
            if panicked {
                return Err(Error::new("ThreadError", "a parallel task panicked"));
            }
            Ok(results)
        })?;
        results.sort_unstable_by_key(|(index, _)| *index);
        if let Some(exit) = results.iter().find_map(|(_, result)| {
            result
                .as_ref()
                .err()
                .filter(|error| error.exit_code().is_some())
        }) {
            return Err(exit.clone());
        }
        Ok(Value::list(
            results
                .into_iter()
                .map(|(_, result)| result)
                .collect::<Result<_>>()?,
        ))
    }
}

fn check_callback(value: &Value, args: usize) -> Result<()> {
    let Value::Function(function) = value else {
        return Err(value.expected("a function"));
    };
    let arity = match &function.kind {
        CallableKind::Script {
            function, receiver, ..
        } => {
            let args = args + usize::from(receiver.is_some());
            if args <= function.parameters.len()
                && function.parameters[args..]
                    .iter()
                    .all(|param| param.default.is_some())
            {
                return Ok(());
            }
            return Err(Error::new(
                "TypeError",
                format!(
                    "{} cannot be called with that many arguments",
                    function.name
                ),
            ));
        }
        CallableKind::Local { arity, .. }
        | CallableKind::Native {
            arity: Some(arity), ..
        } => *arity,
        _ => return Ok(()),
    };
    check_arity(&function.name, arity, args)
}
