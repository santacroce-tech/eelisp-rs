# 11. The engine reads in EELisp

*Code: `read-top` in `src/reader.eel`; `Interpreter::read` and `set_compiled` in
`src/interpreter.rs`; `slice` and the kind argument of `error` in `src/builtins.rs`. Tests:
`tests/reader.rs`, `tests/compiled.rs`.*

## From a function to the engine's reader

Chapter 9 wrote a reader in EELisp and proved it agrees with the Rust one, but the engine didn't
use it. In compiled mode, text still went through `parser::top_forms` before the EELisp compiler
saw it. This chapter switches that. In compiled mode, every piece of text the engine evaluates is
read by `src/reader.eel` and compiled by `src/compiler.eel`, the prelude included.

Chapter 9 named three things missing for this switch: source positions, the right kind of error,
and speed. This chapter does the first two and measures the third.

## What the engine needs from a reader

`eval_str` doesn't only need the forms. For each top-level form it also records its text, exactly
as written, and the comment block above it, so that `(source f)` can show a definition the way
its author wrote it:

```lisp
;; Builds a greeting.
(defn greet (name)
  (str "Hello, " name "!"))    ; used by the welcome screen
```

The Rust reader returns a `TopForm` with three fields: the form, its source text (up to and
including a comment on its last line), and the comment lines directly above it (a blank line ends
the block). So the EELisp reader gained `read-top`, which returns the same three things for each
form:

```lisp
(read-top ";; doc\n(def a 1) ; mine")
→ (((def a 1) "(def a 1) ; mine" ";; doc"))
```

The tokens already carried their start and end positions, so the form's span came for free. The
rest is a port of two small Rust functions: one extends a form's end over a trailing comment, the
other walks back up the lines above it. The Rust originals each fit on a screen, and so do the
EELisp versions.

The differential test now compares `read-top` with `top_forms` field by field (form, text and
comments) on the same megabyte of files, edge cases and random strings. A few new edge cases
cover comments specifically: a blank line between the comment and the form, a trailing comment
separated by a tab, two forms on one line, and Windows line endings.

## The right kind of error

The EELisp reader used to stop with `(error "unclosed list")`, which is a runtime error. The Rust
reader raises a parse error, and a program can tell them apart:

```lisp
(try … (catch e (dict-get e :kind)))   → "parse"
```

So `error` takes an optional kind, `(error "unclosed list" :parse)`. Only the kinds that carry
just a message are accepted (`:parse` and `:syntax`); any other kind stays a runtime error. Now
the two readers' errors are identical in kind as well as message, and the test checks both.

## The switch

The engine holds the reader the way it holds the compiler (chapter 7). It keeps the function
itself, captured when compiled mode is turned on, so a program defining its own `read-top`
doesn't break the engine:

```rust
fn read(&self, src: &str) -> Result<Vec<TopForm>, LispError> {
    match self.reader.borrow().clone() {
        Some(read_top) if self.compiled.get() => /* call it, turn each (form source comments) into a TopForm */,
        _ => parser::top_forms(src),
    }
}
```

`eval_str` and `eval_all` call `self.read`, and nothing else changed. Even the prelude, recompiled
when compiled mode turns on, is now read by the EELisp reader, since the reader is loaded by
then. The whole test suite runs in compiled mode as before, so all 21 suites, `(source f)`
included, now pass with every form read by EELisp.

Two places still use the Rust reader. Both are deliberate:

- **Loading the image itself.** An image is EELisp text, and something has to read the first one
  before any EELisp code exists. This is the seed chapter 10 described.
- **The interpreter.** In interpreted mode nothing changes. That's the mode the editor uses by
  default, and the Rust reader is the reference the EELisp one is tested against.

## What it costs

The first version was much slower on real files than on the compiler's source. Eeditor's snippet
files are about 108,000 characters, mostly long comment blocks documenting each snippet. With call
counting switched on, the first `read-top` took 170 ms to read them, and 1.7 million calls. Counting calls showed why. To copy each form's text,
the reader appended one character at a time with `str`, and every `str` copies everything built
so far. For a form a few thousand characters long, that's quadratic. Strings were built the same
way.

The fix is a general list operation EELisp lacked: `(slice lst start end)`, the list version of
`substr`. A form's text is now `(str-join "" (slice cs start end))`, which is linear. A string
literal with no backslash, which is nearly every string, is cut out the same way, and only
strings with escapes are still built one character at a time.

With that, release build:

| | Rust reader | EELisp reader |
|---|---|---|
| compiled `(+ 1 2)`, per evaluation | 6 µs | 22 µs |
| compiled small `defn`, per evaluation | 19 µs | 54 µs |
| compiled `let` with a `map`, per evaluation | 35 µs | 106 µs |
| load eeditor's snippet files, compiled | 24 ms | 89 ms |
| for reference: load them interpreted | 1.4 ms | (unchanged) |

Reading now costs more than compiling. For a single evaluation that's still tens of microseconds,
far below anything a person typing would notice. Loading every snippet file at startup takes 89
ms instead of 24. That's noticeable, and it's why compiled mode stays opt-in.

The profile says where the rest goes, and it's the same answer as chapter 9: about 1.2 million
calls for 108,000 characters, down from 1.7 million. Each call fetches a character, compares it, or adds one to a
position, and most of them copy a value, which for a string or keyword means allocating.
Chapter 12 takes on that last part: what if copying a string cost no more than copying a number?
