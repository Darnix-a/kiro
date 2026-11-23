use std::collections::BTreeMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::runtime::{file_error, native};
use crate::{Error, Result, Value};

pub(crate) fn module() -> Value {
    Value::map(BTreeMap::from([
        (
            "read_text".into(),
            native("fs.read_text", Some(1), |args| Ok(read(&args[0])?.into())),
        ),
        (
            "read_lines".into(),
            native("fs.read_lines", Some(1), |args| {
                Ok(Value::list(
                    read(&args[0])?.lines().map(Value::from).collect(),
                ))
            }),
        ),
        (
            "write_text".into(),
            native("fs.write_text", Some(2), |args| {
                let path = path(&args[0])?;
                let text = args[1].as_str()?;
                done(path, fs::write(path, text))
            }),
        ),
        (
            "append_text".into(),
            native("fs.append_text", Some(2), |args| {
                let path = path(&args[0])?;
                let bytes = args[1].as_str()?.as_bytes();
                let result = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .and_then(|mut file| file.write_all(bytes));
                done(path, result)
            }),
        ),
        (
            "exists".into(),
            native("fs.exists", Some(1), |args| test_path(&args[0], |_| true)),
        ),
        (
            "is_file".into(),
            native("fs.is_file", Some(1), |args| {
                test_path(&args[0], Metadata::is_file)
            }),
        ),
        (
            "is_dir".into(),
            native("fs.is_dir", Some(1), |args| {
                test_path(&args[0], Metadata::is_dir)
            }),
        ),
        (
            "mkdir".into(),
            native("fs.mkdir", Some(1), |args| {
                let path = path(&args[0])?;
                done(path, fs::create_dir_all(path))
            }),
        ),
        (
            "remove".into(),
            native("fs.remove", Some(1), |args| {
                let path = path(&args[0])?;
                done(path, remove(path))
            }),
        ),
        (
            "copy".into(),
            native("fs.copy", Some(2), |args| {
                let (src, dst) = (path(&args[0])?, path(&args[1])?);
                done(dst, copy(src, dst))
            }),
        ),
        (
            "move".into(),
            native("fs.move", Some(2), |args| {
                let (src, dst) = (path(&args[0])?, path(&args[1])?);
                match fs::rename(src, dst) {
                    Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                        done(dst, copy(src, dst))?;
                        done(src, remove(src))
                    }
                    result => done(dst, result),
                }
            }),
        ),
        (
            "list_dir".into(),
            native("fs.list_dir", Some(1), |args| {
                let path = path(&args[0])?;
                let names = fs::read_dir(path)
                    .map_err(|error| file_error(path, error))?
                    .map(|entry| {
                        let entry = entry.map_err(|error| file_error(path, error))?;
                        utf8(Path::new(&entry.file_name()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(sorted(names))
            }),
        ),
        (
            "glob".into(),
            native("fs.glob", Some(1), |args| {
                let pattern = args[0].as_str()?;
                let paths = glob::glob(pattern)
                    .map_err(|error| Error::new("ValueError", error.to_string()))?
                    .map(|entry| {
                        utf8(&entry.map_err(|error| Error::new("IoError", error.to_string()))?)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(sorted(paths))
            }),
        ),
    ]))
}

fn path(argument: &Value) -> Result<&Path> {
    Ok(Path::new(argument.as_str()?))
}

fn read(argument: &Value) -> Result<String> {
    let path = path(argument)?;
    fs::read_to_string(path).map_err(|error| file_error(path, error))
}

fn done(path: &Path, result: io::Result<()>) -> Result<Value> {
    result.map_err(|error| file_error(path, error))?;
    Ok(Value::Null)
}

fn test_path(value: &Value, test: impl FnOnce(&Metadata) -> bool) -> Result<Value> {
    let path = path(value)?;
    match fs::metadata(path) {
        Ok(metadata) => Ok(Value::Bool(test(&metadata))),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(Value::Bool(false))
        }
        Err(error) => Err(file_error(path, error)),
    }
}

fn utf8(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::new(
            "IoError",
            format!("path is not valid UTF-8: {}", path.display()),
        )
    })
}

fn sorted(mut items: Vec<String>) -> Value {
    items.sort_unstable();
    Value::list(items.into_iter().map(Value::from).collect())
}

fn remove(path: &Path) -> io::Result<()> {
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_dir() {
        fs::remove_dir_all(path)
    } else {
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            if kind.is_symlink_dir() {
                return fs::remove_dir(path);
            }
        }
        fs::remove_file(path)
    }
}

fn copy(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        let parent = dst
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if parent.canonicalize()?.starts_with(src.canonicalize()?) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot copy a directory into itself",
            ));
        }
    }
    copy_entry(src, dst, meta)
}

fn copy_entry(src: &Path, dst: &Path, meta: Metadata) -> io::Result<()> {
    if meta.is_dir() {
        fs::create_dir(dst)?;
        let result = (|| {
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                copy_entry(
                    &entry.path(),
                    &dst.join(entry.file_name()),
                    fs::symlink_metadata(entry.path())?,
                )?;
            }
            fs::set_permissions(dst, meta.permissions())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(dst);
        }
        result
    } else if meta.file_type().is_symlink() {
        copy_link(src, dst)
    } else if meta.is_file() {
        let mut source = File::open(src)?;
        let mut destination = OpenOptions::new().write(true).create_new(true).open(dst)?;
        let result = io::copy(&mut source, &mut destination)
            .and_then(|_| destination.set_permissions(meta.permissions()));
        if result.is_err() {
            drop(destination);
            let _ = fs::remove_file(dst);
        }
        result
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a file, directory, or symbolic link",
        ))
    }
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dst)
}

#[cfg(windows)]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::fs::{FileTypeExt, symlink_dir, symlink_file};
    let target = fs::read_link(src)?;
    if fs::symlink_metadata(src)?.file_type().is_symlink_dir() {
        symlink_dir(target, dst)
    } else {
        symlink_file(target, dst)
    }
}
