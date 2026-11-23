use std::env;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kiro::{Interpreter, Repl, Value};
use rustyline::{DefaultEditor, error::ReadlineError};

pub(crate) fn run() -> ExitCode {
    let mut interpreter = Interpreter::cli();
    interpreter.set_global("args", Value::list(Vec::new()));

    if !io::stdin().is_terminal() {
        let mut source = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut source) {
            eprintln!("kr: {error}");
            return ExitCode::FAILURE;
        }
        return match evaluate(&mut interpreter.repl(), &source) {
            Ok(code) => code.unwrap_or(ExitCode::SUCCESS),
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    match interactive(&mut interpreter.repl()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("kr: {error}");
            ExitCode::FAILURE
        }
    }
}

fn interactive(repl: &mut Repl<'_>) -> rustyline::Result<ExitCode> {
    let mut editor = DefaultEditor::new()?;
    let history = history_path();
    if let Some(path) = &history
        && let Err(error) = editor.load_history(path)
        && !matches!(&error, ReadlineError::Io(error) if error.kind() == io::ErrorKind::NotFound)
    {
        eprintln!("kr: could not load REPL history: {error}");
    }

    let mut source = String::new();
    loop {
        let prompt = if source.is_empty() { "kr> " } else { "... " };
        match editor.readline(prompt) {
            Ok(line) => {
                source.push_str(&line);
                source.push('\n');
                if Repl::needs_more_input(&source) {
                    continue;
                }
                if !source.trim().is_empty() {
                    if editor.add_history_entry(source.trim_end_matches('\n'))?
                        && let Some(path) = &history
                        && let Err(error) = append_history(&mut editor, path)
                    {
                        eprintln!("kr: could not save REPL history: {error}");
                    }
                    match evaluate(repl, &source) {
                        Ok(Some(code)) => return Ok(code),
                        Ok(None) => {}
                        Err(error) => eprintln!("{error}"),
                    }
                }
                source.clear();
            }
            Err(ReadlineError::Interrupted) => source.clear(),
            Err(ReadlineError::Eof) => return Ok(ExitCode::SUCCESS),
            Err(error) => return Err(error),
        }
    }
}

fn evaluate(repl: &mut Repl<'_>, source: &str) -> kiro::Result<Option<ExitCode>> {
    match repl.eval(source) {
        Ok(Value::Null) => Ok(None),
        Ok(value) => {
            println!("{value}");
            Ok(None)
        }
        Err(error) if let Some(code) = error.exit_code() => Ok(Some(ExitCode::from(code))),
        Err(error) => Err(error),
    }
}

fn history_path() -> Option<PathBuf> {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::home_dir().map(|home| home.join(".local/state")))?;
    Some(state.join("kiro/history"))
}

fn append_history(editor: &mut DefaultEditor, path: &Path) -> rustyline::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    editor.append_history(path)
}
