# Kiro

Kiro is a scripting language for automation. A `.kr` file can run commands,
transform their output, and serve an HTTP endpoint.

## Install

```sh
git clone https://github.com/Darnix-a/kiro.git
cd kiro
cargo install --path . --locked
```

Cargo puts `kr` in its bin directory, normally `~/.cargo/bin`. You can also
run `./install.sh` to install into `~/.local/bin`, set up PATH, and install
Rust if needed. Building requires a C compiler and linker.

## Run the monitor

[monitor.kr](monitor.kr) checks Ollama and OpenRouter in parallel. It returns
their status and latency as JSON. It can also search the local CoreX command
catalog through `cx --list`.

```sh
kr monitor.kr --once
kr monitor.kr
```

With the server running, try these in another terminal.

```sh
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/cx/wifi
```

The health route uses the network. The `/cx/` route needs CoreX on PATH.

## Try the REPL

Run `kr` with no arguments. Definitions and imports stay available for the
session. Enter a blank line to submit an indented block, use Up or Ctrl+R
for history, and press Ctrl+D to leave.

You can also run a one-liner or pass Kiro source on stdin.

```sh
kr -e 'print([3, 1, 3, 2] -> unique() -> sort())'
printf '%s\n' 'print("hello")' | kr
```

Read the [language reference](LANGUAGE.md) for command literals, pattern
matching, structs, and the built-in modules.

## CoreX and editors

Run `./install.sh --embed` to install the shared library for CoreX. With a
CoreX checkout that supports the Kiro bridge, use `cx kiro script.kr` for a
foreground action or `cx bg run script.kr` for a background script. The
[embedding example](examples/embed.rs) shows direct Rust integration.

Run `./editors/install.sh vscode` from the checkout to install VS Code
highlighting. [Editor setup](editors/README.md) also covers VSIX installation,
Kate, Vim, and Neovim.

## Development

```sh
cargo test --locked --all-targets
cargo run --locked --example embed
```
