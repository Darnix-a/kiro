use std::env;
use std::path::Path;
use std::process::ExitCode;

use kiro::Interpreter;

mod repl;

const USAGE: &str = "Usage: kr                              Start the REPL\n       kr <script.kr> [arguments...]\n       kr -e <code> [arguments...]";

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(script) = arguments.next() else {
        return repl::run();
    };
    if script == "--help" || script == "-h" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if script == "--version" {
        println!("Kiro {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let inline = if script == "-e" {
        let Some(code) = arguments.next() else {
            eprintln!("kr: -e requires Kiro source code\n{USAGE}");
            return ExitCode::from(2);
        };
        let Ok(code) = code.into_string() else {
            eprintln!("kr: inline source must be valid UTF-8");
            return ExitCode::from(2);
        };
        Some(code)
    } else {
        None
    };
    let mut interpreter = Interpreter::cli();
    interpreter.set_global(
        "args",
        kiro::Value::list(
            arguments
                .map(|argument| argument.to_string_lossy().into_owned().into())
                .collect(),
        ),
    );
    let result = match &inline {
        Some(code) => interpreter.run_source(code, "<inline>"),
        None => interpreter.run_file(Path::new(&script)),
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) if let Some(code) = error.exit_code() => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
