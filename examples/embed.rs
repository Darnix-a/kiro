use kiro::{Interpreter, Result};

fn main() -> Result<()> {
    let mut interpreter = Interpreter::new();
    interpreter.define_function("document_title", 0, |_| Ok("Quarterly report".into()));

    let report = interpreter.run_source(include_str!("embedded.kr"), "embedded.kr")?;
    println!("{report}");
    Ok(())
}
