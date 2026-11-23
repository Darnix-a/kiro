# Kiro language reference

Run a script with `kr script.kr`. Extra arguments appear in the script as the
`args` list, starting at `args[0]`. The examples below use the CLI prelude.

Use `kr -e` to run inline source. Quote the source for your shell and pass
script arguments after it.

```sh
kr -e 'print("Hello, {args[0]}")' from-terminal
```

`exit(code?)` stops execution immediately with a status from 0 through 255.
It defaults to 0. It passes through `catch` and `retry` to the CLI.

## Interactive use

Run `kr` with no arguments to open the REPL. It echoes the value of the
last expression and keeps names and imports between submissions. Finish an
indented block with a blank line. A multiline list, map, or call continues
until its delimiters close. Up and Ctrl+R recall history, Ctrl+C clears the
current input, and Ctrl+D leaves the REPL.

History lives in `$XDG_STATE_HOME/kiro/history` or
`~/.local/state/kiro/history`. With redirected stdin, `kr` reads and runs
the entire input as one submission.

## Statements and values

One statement goes on each line. A colon opens a block and indentation marks
its body. Use spaces to indent. Empty lines and lines beginning with `#` are
ignored. You can spread a list, map, call, or parenthesized expression across
lines.

```text
name = "Kiro"
count = 0

if count == 0:
    count = count + 1
    print("Hello from {name}")
```

Assignment creates or updates a variable in the current function or file.
Indented blocks share that scope. If a block never ran and it was the only
place that assigned a variable, reading the variable raises `NameError`.

| Value | Examples and behavior |
| --- | --- |
| Null | `null` represents an absent value. |
| Boolean | `true` and `false` are the values used in conditions. |
| Integer | `42` is a signed 64-bit integer. Integer overflow raises `OverflowError`. |
| Float | `3.5` and `1e-3` are 64-bit floats. Division with `/` returns a float. |
| String | `"hello"` and `'hello'` hold UTF-8 text. Indexes and `len` count Unicode characters. |
| List | `[1, 2, 3]` is ordered. Calling `list.append(value)` mutates the list. |
| Map | `{name: "Kiro", "build-id": 42}` maps string keys to values. |
| Struct | `Job(title: "sync")` creates a record declared with `struct`. |
| Enum member | `Status.Active` is a distinct value with a member name. |
| Function | A named `fn` or an expression such as `x => x * 2` can be passed to another function. |
| Command result | A captured command provides `out`, `err`, `code`, and `ok`. |

Use zero-based indexes for lists and strings. A negative index counts from
the end, so `items[-1]` gets the last item. A slice `items[start:end]`
includes the start and excludes the end. Omit either bound to use the
beginning or end. Bounds are clipped to the sequence, and string indexes
and slices count Unicode characters.

Read a map with `map.name` or
`map["build-id"]`. A missing key raises `KeyError` and an out-of-range index
raises `IndexError`. Assigning a list to a second variable shares that list.

Double-quoted strings evaluate expressions inside braces in the current
scope. Use a variable with `{name}`, read a field with `{err.kind}`, or call
a function with `{len(matches)}`. An expression can use the usual Kiro quotes
inside the braces. Single quotes keep braces as text. In double quotes, write
`\{name}` to spell the placeholder literally. Strings also support `\n`,
`\r`, `\t`, escaped quotes, and `\\`.

```text
matches = ["wifi-main", "wifi-guest"]
print("Saved {len(matches)} matches. First match: {matches[0]}")
```

| String method | Result |
| --- | --- |
| `text.trim()` | Removes leading and trailing whitespace. |
| `text.split(sep)` | Splits on a nonempty separator and keeps empty fields. |
| `text.replace(old, new)` | Replaces all occurrences of `old`. |
| `text.starts_with(prefix)` | Checks the prefix. |
| `text.ends_with(suffix)` | Checks the suffix. |
| `text.lower()` and `text.upper()` | Converts Unicode letter case. |
| `sep.join(list)` | Joins a list of strings with `sep` between them. |

## Operators and control flow

From tighter to looser binding, the operator groups are calls and member
access, unary `-`, `* / %`, `+ -`, `< <= > >= in contains`, `== !=`, `not`,
`and`, `or`, `??`, and `->`. Use parentheses to choose a different grouping.
`+` also concatenates strings.

`value ?? fallback` evaluates the fallback only when the value is `null`.
`value?.field` returns `null` for a null receiver or an absent field. Chain
`?.` at each access that can be absent. Optional method calls skip their
arguments when the receiver or member is absent.

```text
settings = {network: {host: "localhost"}}
print(settings?.network?.host ?? "unknown")
```

`needle in collection` and `collection contains needle` test the same thing.
They check substrings in strings, elements in lists, and keys in maps.
Conditions and logical operators require booleans. `and` and `or`
short-circuit.

```text
fn failed_names(checks):
    names = []
    for check in checks:
        if check.failed:
            names.append(check.name)
    return names

checks = [
    {name: "cache", failed: true},
    {name: "config", failed: false},
]
print(failed_names(checks))
```

Use `elif` and `else` for additional branches. `while condition:` repeats as
long as the condition is true. A `for` loop visits list elements, map keys,
string characters, or the output lines of a captured command. Use `break`
to leave a loop, `continue` to skip to the next iteration, or `pass` for an
empty block.

Functions have their own locals and can read names in their enclosing scope.
Assigning a name inside a function makes it local to that function. A function
that finishes without `return` produces `null`. Anonymous functions return
the value of the expression after `=>`. Write `line => "wifi" in line` for
one parameter, `() => time.now()` for zero, or `(total, x) => total + x` for
more.

Named functions can declare defaults. Keyword arguments use either
`name: value` or `name=value`. Put positional arguments first. Defaults
evaluate on each call when the argument is omitted and can refer to earlier
parameters. Script functions, methods, and struct constructors accept
keyword arguments. Standard native functions take positional arguments.

```text
fn connect(host, port=8080, protocol="http"):
    return "{protocol}://{host}:{port}"

print(connect("localhost"))
print(connect("example.com", protocol: "https", port: 443))
```

## Structs and enums

Declare a struct's fields one per line. A field is required unless it has a
default. Constructors evaluate omitted defaults for each instance, so
instances can each receive their own list. A field annotation checks its
value at construction. Methods take `self` as their first parameter and
calls on an instance supply it automatically.

```text
enum Status:
    Active
    Inactive

struct Job:
    name: String
    port: Int = 8080
    status: Status = Status.Active
    tags: List = []
    notes = null

    fn label(self, prefix="job"):
        return "{prefix} {self.name}:{self.port}"

job = Job(name: "sync")
job.tags.append("nightly")
print(job.label(prefix: "scheduled"))
```

Built-in annotation names include `Any`, `Int`, `Float`, `Number`, `String`,
`Bool`, `List`, `Map`, and `Function`. Use a struct or enum name to require
a value of that declaration. Instances have fixed fields and can hold
mutable lists. Enums declare one member per line. Read a member's short name
with `Status.Active.name`. When serialized to JSON, a struct becomes an
object of its fields and an enum member becomes its short name.

## Matching and destructuring

`match` evaluates the subject once and runs the first `case` whose pattern
and optional `if` guard succeed. A bare name captures a value and `_` ignores
it. Use literals or qualified enum names for comparisons. Join alternatives
with `|`. Alternatives must bind the same names.

```text
payload = {status: 200, body: "ready"}
match payload:
    case {status: 200, body} if body != "":
        print(body)
    case {status: 400 | 404}:
        print("request failed")
    case _:
        print("unrecognized response")

[head, ...tail] = [1, 2, 3]
{host, port: service_port} = {host: "localhost", port: 8080}
print(head, tail, host, service_port)
```

List patterns match exactly unless the final item is `...rest`. Map
patterns require their listed keys and allow extra keys. In a map pattern,
`{host}` is shorthand for `{host: host}`. A successful branch exposes its
bindings in the surrounding function or file. A failed match or guard
leaves those names alone. Destructuring assignment raises `PatternError`
when the value fails to match and applies its bindings only after the whole
pattern succeeds.

## Comprehensions and ranges

Use `[value for item in sequence if cond]` for a list and
`{key: value for item in sequence if cond}` for a map. The condition is
optional and must return a boolean when supplied. The iteration variable
is local to the comprehension. Map keys must be strings. When a key repeats,
the last entry wins.

```text
numbers = range(0, 6)
evens = [n * n for n in numbers if n % 2 == 0]
labels = {"item-{n}": n for n in evens}
print(labels, evens[-2:])
```

`range(start, end, step?)` returns an integer list. The end is exclusive and
the step defaults to 1. Use a negative step to count down. The step must be
nonzero, and a range can contain up to one million elements.

## Command literals

Use `$ "command arguments"` or a backtick-delimited command. A dollar sign
before the opening backtick is optional. Quote any argument that includes
spaces. Interpolate an expression such as `{entry.name}` or `{len(matches)}`
to insert its string, boolean, or numeric result into an argument.

```text
entry = {message: "hello from Kiro"}
$ "printf '%s' {entry.message}"
```

Kiro splits the literal into arguments before evaluating any interpolation.
An expression can contain spaces, and a result containing spaces or shell
characters stays in that argument. Single and
double quotes inside the command group text into one argument, including an
empty argument when the quotes are empty. Use `\{` for a literal opening
brace or a backslash outside quotes to escape the next character. Inside
single quotes, backslashes otherwise stay literal. Inside double quotes,
a backslash can escape another backslash or a double quote. When the outer
command delimiter is double quotes, write `\"` to put a double-quote
delimiter around an argument inside it.

Commands launch the executable directly. Shell syntax such as `|`, `>`,
globs, and shell variable expansion has no special meaning in the command
literal. To run shell syntax intentionally, call an executable such as
`sh` with a script or a `-c` argument.

### Streaming and capturing

| Use | Behavior |
| --- | --- |
| A command on its own line | Inherits stdin, stdout, and stderr, so output and terminal escape sequences reach your terminal as the tool runs. A nonzero exit raises `ProcessError`. |
| A command in an assignment, call, or pipeline | Captures output and returns a result on any exit status. Inspect `result.ok` or `result.code` to handle failure. |

Both forms raise `ProcessError` if the executable cannot start. Captured
outcomes have these fields.

| Field | Value |
| --- | --- |
| `out` | Stdout as text with trailing CR and LF characters removed. |
| `err` | Stderr as text, including trailing line breaks. |
| `code` | The exit code, or `null` if there is no numeric status. |
| `ok` | `true` if the process succeeded. |

Use `result.lines()` or `lines(result)` to split stdout into a list. Strings
also support `text.lines()` and `lines(text)`.

```text
out = $ "printf '%s\n' wifi-main wired wifi-guest"
for line in out.lines():
    if "wifi" in line:
        print("Found module: {line}")
```

For a programmatically constructed argument list, use
`process.run(executable, arguments)`. It captures output in `stdout`,
`stderr`, and `exit_code`, keeps trailing line breaks, and raises an error
on a nonzero exit.

## Pipelines

`value -> f(a, b)` calls `f(value, a, b)`. The input runs first and each stage
runs from left to right. You can omit parentheses for a function with no
extra arguments. Put parentheses around a multiline pipeline.

```text
matches = (
    $`printf '%s\n' wifi-main wired wifi-guest`
    -> lines()
    -> filter(line => line contains "wifi")
)
matches -> print()
```

A pipeline passes Kiro values between functions. A command captures its
output before the next stage runs.

| Function | Result |
| --- | --- |
| `filter(values, fn)` | Keeps items for which `fn(item)` returns `true`. |
| `map(values, fn)` | Returns the result of applying `fn` to each item. |
| `reduce(values, init, fn)` | Folds left to right with `fn(accumulator, item)`. An empty input returns `init`. |
| `any(values, fn)` | Stops at the first `true`. An empty input returns `false`. |
| `all(values, fn)` | Stops at the first `false`. An empty input returns `true`. |
| `sort(values, key_fn?)` | Sorts a new list in ascending order using numbers or strings as keys. Ties keep input order. The optional function computes each key once. |
| `unique(values)` | Keeps the first occurrence of each distinct value. |

These helpers use the same iteration order as `for`. Predicates must return a
boolean.

```text
total = [3, 1, 3, 2] -> unique() -> sort() -> map(x => x * 2) -> reduce(0, (acc, x) => acc + x)
print(total)
```

## Parallel work and retries

`parallel(tasks)` runs a list of zero-argument functions on worker threads and
returns their results in input order. It waits for all tasks to finish and
propagates the first error in input order. An `exit` from a task exits the
script after the group finishes. Workers share captured values and the
interpreter's environment. The order of side effects across tasks depends
on thread scheduling.

```text
results = parallel([
    () => ($ "printf '%s' first").out,
    () => ($ "printf '%s' second").out,
])
print(results)
```

`retry(attempts, delay_ms, fn)` calls a zero-argument function up to
`attempts` times. It returns the first successful result, waits `delay_ms`
between errors, and raises the last error if all attempts fail. The count
must be positive and the delay must be a nonnegative integer. Use
`process.run` to retry nonzero command exits. A captured command result or an
HTTP error status is a normal return value.

```text
result = retry(3, 200, () => process.run("cat", ["status.txt"]))
print(result.stdout)
```

## Prelude and modules

The CLI makes these names available in every source file. You can also
explicitly import a standard module, including under an alias.

| Name | What it provides |
| --- | --- |
| `print(values...)` | Prints values separated by spaces and ends the line. |
| `input(prompt?)` | Writes an optional prompt and reads one line, dropping its line ending. Returns `null` at EOF. |
| `len(value)` | Counts list elements, map keys, or string characters. |
| `lines(value)` | Splits a string or command result into lines. |
| `process.run(executable, arguments)` | Captures stdout and stderr and checks the exit status. |
| `env.get(name)` | Reads an environment variable as text, or returns `null` when it is absent. |
| `env.set(name, value)` | Sets a string value in this interpreter's environment. Child commands inherit it. |
| `env.all()` | Returns a map of available UTF-8 environment entries with the interpreter's changes applied. |
| `range(start, end, step?)` | Builds an integer list with an exclusive end. |
| `time.sleep(ms)` | Pauses for a nonnegative number of milliseconds. |
| `time.now()` | Returns the current Unix timestamp in milliseconds. |
| `time.format(ts, fmt?)` | Formats a millisecond timestamp in UTC. Defaults to ISO 8601 with milliseconds. Pass a [strftime pattern](https://docs.rs/chrono/0.4.45/chrono/format/strftime/index.html) to customize it. |
| `json.parse(text)` | Parses JSON into Kiro values. JSON integers must fit in 64 signed bits. |
| `json.stringify(value, pretty?)` | Encodes JSON-compatible values, including command result fields. Pass `true` for two-space indentation and line breaks. Omit the flag or pass `false` for compact JSON. |

```text
name = input("Job name? ")
if name == null:
    exit(1)
env.set("JOB_NAME", name.trim())
print("{time.format(time.now(), "%Y-%m-%d %H:%M:%S")} {env.get("JOB_NAME")}")
```

## Filesystem

Filesystem paths are relative to the working directory unless they are
absolute. Text operations use UTF-8. Mutating operations return `null`.

| Function | Behavior |
| --- | --- |
| `fs.exists(path)` | Checks whether the path has a target. |
| `fs.is_file(path)` | Checks for a regular file. |
| `fs.is_dir(path)` | Checks for a directory. |
| `fs.mkdir(path)` | Creates missing parent directories and accepts an existing directory. |
| `fs.remove(path)` | Deletes a file or link, or recursively removes a directory. |
| `fs.copy(src, dst)` | Copies a file, link, or directory tree to a new destination. |
| `fs.move(src, dst)` | Renames a path using the platform's replacement rules. Across filesystems it copies to a new destination, then removes the source. |
| `fs.list_dir(path)` | Returns the immediate entry names in sorted order. |
| `fs.glob(pattern)` | Returns matching paths in sorted order. Supports patterns such as `logs/*.log` and `logs/**/*.log`. |
| `fs.read_text(path)` | Reads a file into one string. |
| `fs.read_lines(path)` | Returns the lines of a file without their line endings. |
| `fs.write_text(path, text)` | Writes text, replacing any previous contents. |
| `fs.append_text(path, text)` | Appends text and creates the file if needed. Include a newline when appending a full line. |

```text
fs.mkdir("logs")
fs.append_text("logs/run.log", "{time.format(time.now())} started\n")
for path in fs.glob("logs/*.log"):
    print(path, len(fs.read_lines(path)))
```

## Regex, paths, math, and randomness

| Function | Result |
| --- | --- |
| `regex.match(pat, text)` | Returns whether the pattern occurs. Use `^` and `$` to anchor it. |
| `regex.find_all(pat, text)` | Returns the matched substrings in order. |
| `regex.replace(pat, text, repl)` | Replaces all matches. Use `$1` for a numbered capture in the replacement. |
| `path.join(parts...)` | Joins path components using the platform's path syntax. An absolute component starts a new path. |
| `path.basename(p)` | Returns the final name or an empty string if there is none. |
| `path.dirname(p)` | Returns the parent directory. A bare relative name has parent `"."`. |
| `path.ext(p)` | Returns the final extension without its dot, or an empty string. |
| `math.floor(x)`, `math.ceil(x)`, `math.round(x)` | Return integers. `round` rounds halfway values away from zero. |
| `math.abs(x)`, `math.sqrt(x)` | Return the absolute value or square root. |
| `math.min(...)`, `math.max(...)` | Accept numbers or one list of numbers. |
| `random.int(min, max)` | Samples an integer with both bounds included. |
| `random.float()` | Samples a float from 0 inclusive to 1 exclusive. |
| `random.choice(list)` | Samples an item from a nonempty list. |

The [regex syntax reference](https://docs.rs/regex/1.13.2/regex/#syntax)
lists the supported expressions.

## HTTP

| Function | Request |
| --- | --- |
| `http.get(url, headers?)` | Sends an HTTP or HTTPS GET request. |
| `http.delete(url, headers?)` | Sends a DELETE request. |
| `http.post(url, body, headers?)` | Sends a POST request with a body. |
| `http.put(url, body, headers?)` | Sends a PUT request with a body. |
| `http.patch(url, body, headers?)` | Sends a PATCH request with a body. |

HTTP request headers are an optional string-to-string map. A map or list
body is encoded as JSON and defaults to `application/json`. A string body
defaults to `text/plain; charset=utf-8`. Supply a content-type header to
override either default. The response has `status`, `ok` (true for 2xx),
`body`, and `headers`. Use `response.json()` to parse its body. Use lower-case names to
read response headers, for example `response.headers["content-type"][0]`.
Header values are lists so repeated headers are preserved. A 4xx or 5xx
response still gives you a response object. Network and decoding failures
raise `HttpError`. Connection setup has a 3-second limit, shared
across address attempts and the TLS handshake. If both IPv4 and IPv6 are
available and the preferred connection stalls, Kiro starts trying the
other family after 300 ms. The full request has a 30-second timeout and
a 10 MiB body limit.

```text
fn post_report(url, report):
    return http.post(url, report).json()
```

For a human-readable file, pass the pretty flag when you serialize the report.

```text
report = {matches: ["wifi-main", "wifi-guest"]}
fs.write_text("report.json", json.stringify(report, true))
```

### Serving requests

`http.serve(port, handler)` binds an HTTP server to `127.0.0.1` and calls the
handler once for each request. Use a port from 1 through 65535. The call
keeps serving until the script exits. Handlers run one at a time.

The request map contains `method`, `path`, `headers`, `body`, and `query`.
The path excludes the query string. Query values are URL-decoded strings,
with the last value kept for repeated keys. Header names are lowercase and
their values are lists. The body is UTF-8 and can be up to 10 MiB.

Return `{status, headers?, body}` as a Kiro map. A status must be from 200
through 599. A map or list body becomes JSON. A string becomes text, and
`null` sends an empty body. Optional response headers can contain strings or
lists of strings. A handler error sends HTTP 500 and the server continues.

```text
fn handle(req):
    match req.path:
        case "/status" if req.method == "GET":
            return {status: 200, body: {ready: true}}
        case _:
            return {status: 404, body: "not found"}

http.serve(8080, handle)
```

## Imports

Load another source file by path, relative to the file containing the import.
The imported file can expose functions and assignments with `export`.

```text
# helpers.kr
export fn report_name():
    return "failures.json"
```

```text
# main.kr
import "./helpers.kr" as helpers
print(helpers.report_name())
```

## Errors and embedding

A runtime error stops the script unless a surrounding `try` catches it.
`catch err:` binds a map with `kind` and `message`. Errors that have a
source location include `path`, `line`, and `column`. A failed
`process.run` also supplies `command`, `args`, `exit_code`, `stdout`, and
`stderr`. A streaming command has the same process fields, with empty output
fields because its output went straight to the terminal.

```text
try:
    process.run("sh", ["-c", "exit 7"])
catch err:
    print(err.kind, err.exit_code)
```

In Rust, create a host with `kiro::Interpreter::new()`. Use
`define_function` to expose host functions, `set_global` to provide data,
and `enable_standard_module` for each built-in module you want scripts to
use. Use `define_thread_safe_function` for callbacks that should also run
inside `parallel`. Enabling `process` also enables command literals. Hosts can provide
their own modules with `add_module` and enable source imports separately
with `allow_file_imports`. The entry script's top-level `return` is the Rust
result value. An `exit` ends the current execution, and the returned error
exposes the requested status through `exit_code()`. See the
[embedding demo](examples/embed.rs) for a complete example.
