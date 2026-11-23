use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char};
use std::panic::catch_unwind;

use serde_json::{Value as Json, json};

use crate::{Error, Interpreter, Result, runtime::native, standard};

#[unsafe(no_mangle)]
pub extern "C" fn kiro_embed_api_version() -> u32 {
    1
}

/// Both inputs must be readable NUL-terminated UTF-8 strings.
/// Release the returned JSON string with kiro_free_string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiro_run_corex_file(
    path: *const c_char,
    context: *const c_char,
) -> *mut c_char {
    let run = catch_unwind(|| {
        // The embedding caller keeps both strings alive until the call returns.
        if path.is_null() || context.is_null() {
            return Err(Error::new(
                "EmbedError",
                "expected a script path and context",
            ));
        }
        let read_error = |error: std::str::Utf8Error| Error::new("EmbedError", error.to_string());
        let path = unsafe { CStr::from_ptr(path) }
            .to_str()
            .map_err(read_error)?;
        let context = unsafe { CStr::from_ptr(context) }
            .to_str()
            .map_err(read_error)?;
        corex_file(path, context)
    });
    let result = match run {
        Ok(Ok(value)) => json!({"code": 0, "value": value}),
        Ok(Err(error)) if let Some(code) = error.exit_code() => {
            json!({"code": code, "value": null})
        }
        Ok(Err(error)) => json!({"code": 1, "error": error.to_string()}),
        Err(_) => json!({"code": 1, "error": "Kiro runtime panicked"}),
    };
    CString::new(result.to_string()).unwrap().into_raw()
}

/// Safety: result is null or was returned by kiro_run_corex_file and has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiro_free_string(result: *mut c_char) {
    if !result.is_null() {
        drop(unsafe { CString::from_raw(result) });
    }
}

fn corex_file(path: &str, context: &str) -> Result<Json> {
    let context = standard::parse_json(context)?;
    let args = context.field("args")?;
    let commands = context.field("commands")?;
    let catalog = commands.clone();
    let has_command = native("cx.has_command", Some(1), move |args| {
        catalog.contains(&args[0])
    });
    let mut interpreter = Interpreter::cli();
    interpreter.set_global("args", args.clone());
    interpreter.add_module(
        "cx",
        BTreeMap::from([
            ("args".into(), args),
            ("command".into(), context.field("command")?),
            ("commands".into(), commands),
            ("version".into(), context.field("version")?),
            ("has_command".into(), has_command),
        ]),
    );
    standard::to_json(&interpreter.run_file(path)?, 0)
}
