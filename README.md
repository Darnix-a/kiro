# Kiro

Kiro is a scripting language for running commands, working with files, and
automating apps. Scripts use `.kr` and run with `kr`.

## Install

```sh
git clone https://github.com/Darnix-a/kiro.git
cd kiro
./install.sh
```

The installer builds Kiro from source and puts `kr` in `~/.local/bin`. It
installs Rust with [rustup](https://rustup.rs/) if Cargo is missing and adds
the bin directory to your shell config. You'll need a system linker (usually
provided by your distro's C compiler package) and `curl` if Rust needs to
be installed.

Open a new terminal and run `kr --version`. For a different install location,
use `./install.sh --prefix /some/path`. Pass `--no-path` to skip shell setup.

## A script

Save this as `hello.kr` and run `kr hello.kr`.

```text
import process

result = process.run("printf", ["%s", "hello from Kiro"])
print(result.stdout)
```

`process.run` takes an argument list and captures stdout and stderr. A failed
command raises a `ProcessError`. Catch it with `try:` and `catch err:` when
you need to inspect `err.exit_code` or `err.stderr`.

The [report example](examples/report.kr) uses a command's JSON output to write
a list of failed checks to `failures.json`.

```sh
kr examples/report.kr examples/checks.json
```

When a script grows, put helpers in another file and load them with
`import "./helpers.kr" as helpers`. Mark the functions you want to expose
with `export fn`.

## Embedding

The CLI uses the same Rust library as the [embedding example](examples/embed.rs).
An app can register its own functions, enable the standard modules it needs,
and get a value back from a script's top-level `return`.

```sh
cargo run --locked --example embed
```

## Development

```sh
cargo run --locked -- examples/report.kr examples/checks.json
cargo test --locked --all-targets
```

The installer and examples are tested on Linux.
