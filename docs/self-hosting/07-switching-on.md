# 7. Switching it on

*Code: `Interpreter::set_compiled` in `src/interpreter.rs`, `boot::IMAGE` in `src/boot.rs`, the
`--compiled` flag in `src/bin/eelisp.rs`. Tests: `tests/compiled.rs`, and every other test file
run a second time with `EELISP_COMPILED=1`.*

## One switch

Until now the compiler was something you called: `(compile-run form)`. This step makes it the
engine's own way of evaluating, behind a switch:

```rust
let it = Interpreter::new();
it.set_compiled(true)?;
it.eval_str("(defn sq (x) (* x x)) (sq 12)")?;   // compiled, run on the VM
```

The first time the switch turns on, three things happen:

1. **The compiler is loaded from its bootstrap image.** The image (chapter 4) is now embedded in
   the library (`boot::IMAGE`), so turning compiled mode on never interprets the compiler's
   source. It is bytecode from the first instruction.
2. **The prelude is recompiled.** `take`, `last`, `when`, `unless`… were defined by the
   interpreter when the engine started. Recompiling them makes them bytecode too, macros
   included.
3. **The engine keeps the `compile` function itself**, not its name. A program is free to define
   its own `compile`, a perfectly reasonable name for a user's function, without breaking the
   engine.

After that, each top-level form goes through the compiler and runs on the VM.

## Where the switch sits

The interpreter's public entry points (`eval_str`, `eval_all`, `call`) keep their shape. Only the
one line that evaluates a form changes:

```rust
fn eval_form(&self, form: &Value) -> Result<Value, LispError> {
    if self.compiled.get() {
        self.compile_and_run(form)
    } else {
        eval::eval(form.clone(), self.global.clone())
    }
}
```

That placement matters. Everything that happens *around* evaluating a form keeps working:
recording each definition's source for `(source f)`, and the recalculation of sheets whose
formulas read a table that was just written (a feature that landed while this work was going
on). Hosts don't change either: the CLI, the `--serve` protocol, `EngineHandle` and the editor
all call the same functions as before.

## Three ways in

- **A host** calls `set_compiled(true)`. With `EngineHandle`, that goes in the setup closure that
  already runs on the engine's thread, so no API had to change:

  ```rust
  let engine = EngineHandle::spawn_with(db_path, |it| it.set_compiled(true).unwrap());
  ```

- **The CLI** takes `--compiled` with any of its forms: a file, `-e`, the REPL, `--serve`.
- **The environment variable** `EELISP_COMPILED=1` starts *every* engine in compiled mode. It
  exists mainly for the next section, and it lets anyone try compiled mode in a host without
  changing the host.

## The strongest test we have

The differential test (chapter 3) compares the two engines on a corpus of snippets. With a
switch on the engine itself, a stronger test became possible: run **the entire test suite**
in compiled mode.

```
EELISP_COMPILED=1 cargo test --all
```

Every engine every test creates now compiles its forms. That includes the database, the agenda,
sheets with their recalculation, the host JSON boundary, the docs and `(source …)`, the threaded
`EngineHandle`, and HTTP. All 20 test suites pass, with and without the database built in.

One test needed a change, and it's instructive. It checked the exact text of some bytecode,
labels included. In compiled mode the engine's own compiler had already numbered labels while
compiling the prelude, so the numbers started higher. Labels are arbitrary names, so the test now
resets the counter first. It was the only test that depended on the compiler's internal state.

A separate file, `tests/compiled.rs`, checks the mode itself rather than trusting the green run.
Definitions really become bytecode. The prelude's functions and macros really are compiled. The
switch goes both ways. A user's own `compile` doesn't break anything. The compiler's internal
`compiler-…` names stay out of `(functions)`. An `EngineHandle` can start compiled. And
`EELISP_COMPILED=1` really does reach every new engine.

CI now runs the suite twice, interpreted and compiled, so neither path can regress unnoticed.

## What it costs

Compiled code runs faster, but every evaluation now starts by compiling. For an editor, where
people type small forms and the app loads its snippet files at startup, that cost matters as much
as running speed. So the benchmark gained a table for it (release build):

| | interpreter | compiled |
|---|---|---|
| start an engine | 0.15 ms | 2.8 ms |
| evaluate `(+ 1 2)` | 0.4 µs | 12 µs |
| evaluate a small `defn` | 1.1 µs | 30 µs |
| evaluate a `let` with a `map` | 2.3 µs | 67 µs |
| load all of eeditor's snippet files | 1.5 ms | 54 ms |

Running the code itself is 1.0–1.7× faster compiled. Compiling it costs tens of microseconds per
form. A single evaluation stays far below anything a person can notice, but loading a few hundred
definitions at startup goes from instant to a twentieth of a second.

That's why compiled mode is a switch, and off by default. It's also a clear next target, and a
fitting one: the compiler is an EELisp program, and its slowness is ordinary program slowness. It
builds instruction lists with repeated `append`, which copies; it finds variables by walking
lists; it keeps its context in dictionaries. Making the compiler faster is optimizing an EELisp
program with EELisp's own tools. Every gain also speeds up the bootstrap, since that's the compiler
compiling itself.

## Still interpreted

Three things don't go through the compiler yet:

- `(eval form)`: since chapter 6 it runs at the top level, but on the interpreter.
- Agenda rules: their conditions and actions are evaluated from Rust.
- Sheet formulas: likewise.

They give the same results either way. Moving them over is a choice for later, once compiling is
cheap enough to do on every rule and every recalculation.
