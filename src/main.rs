use std::env;
use std::path::Path;
use std::process::ExitCode;

use kiro::Interpreter;

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(script) = arguments.next() else {
        eprintln!("Usage: kr <script.kr> [arguments...]");
        return ExitCode::from(2);
    };
    if script == "--help" || script == "-h" {
        println!("Usage: kr <script.kr> [arguments...]");
        return ExitCode::SUCCESS;
    }
    if script == "--version" {
        println!("Kiro {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let mut interpreter = Interpreter::cli();
    interpreter.set_global(
        "args",
        kiro::Value::list(
            arguments
                .map(|argument| argument.to_string_lossy().into_owned().into())
                .collect(),
        ),
    );
    match interpreter.run_file(Path::new(&script)) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
