# 12. Sharing strings

*Code: `Value` in `src/value.rs`, and every place that makes or reads a string (about 250 of
them). First on its own branch, `exp/shared-strings`. The numbers come from `examples/bench.rs` and a few timing
scripts, run against the previous commit on the same machine.*

## The finding

Chapters 9 and 11 profiled the EELisp reader and found time going to `malloc`, `free` and
copying. The cause wasn't the reader. In this engine a string value *owned* its text:

```rust
pub enum Value {
    Str(String),        // copying it copies the text
    Keyword(String),    // so does copying a keyword
    …
}
```

Every time the VM copies a value (loading a variable, passing an argument, storing it in a list,
returning it), a string or keyword was duplicated: a fresh allocation, a copy of its bytes, and
later a free. For a one-character string that's mostly overhead. The reader handles one-character
strings by the hundred thousand.

Symbols had already been fixed this way, long before this work started. A symbol is a `Sym`, a
pointer to a name stored once (an "interned" name), so copying one costs a reference count.

## The change

EELisp strings are immutable: no builtin changes a string in place. So nothing ever needs its own
copy of a string's text, and every holder can share one:

```rust
pub enum Value {
    Str(Rc<str>),       // shared text: copying it is a reference-count bump
    Keyword(Sym),       // interned, like symbols
    …
}
```

- **Strings** become `Rc<str>`: one allocation holding the text, shared by every value that holds
  it, and freed when the last holder drops it.
- **Keywords** become interned, exactly like symbols. That's what Lisps have always done with
  them. `:name` is one object, so comparing two keywords compares two pointers.

The type change itself is two lines. Its consequences touched about 250 places in the engine
that build strings from Rust `String`s or read them as one. Most were mechanical. Rust's compiler
reports each mismatch with the expected and found types, and a small script applied the obvious
conversion (`.into()` one way, `.to_string()` the other) at each reported position. The rest, about
twenty, needed a person: patterns like `Value::Keyword(k) | Value::Str(k)`, which used to bind the
same type in both arms and no longer do.

Two hot builtins were then rewritten for the new type. `str-chars` builds each one-character
string in place, with no intermediate `String`. `substr` now slices the text directly instead of
first collecting every character into a vector.

## A surprise: smaller was slower

The first measurement was a mixed bag. Text-heavy code was much faster, as expected, but compiled
loops over numbers, which never touch a string, were 5–9% *slower*. A pure numeric loop:

| | before | shared strings |
|---|---|---|
| `(loop … (recur (+ i 1) (+ acc (* i 2))))` × 100 | 0.96 s | 1.05 s |

The profile showed more time dropping values, but nothing had changed in the VM's code. What had
changed was the size of `Value`. A `String` is 24 bytes (pointer, length, capacity), so `Value`
was 32 bytes. An `Rc<str>` is 16 (pointer, length), so `Value` shrank to **24 bytes**.

A smaller value sounds strictly better: less memory, more values per cache line. The way to test
it was to put the size back. With a dummy variant padding `Value` to 32 bytes again, the loop ran
in 0.95 s. So the slowdown came from the size, not from sharing strings.

The VM keeps values in arrays (its stack and its frames' slots) and finds value number *i* at
*i × size*. With 32 bytes that's a shift; with 24 it's a multiplication. It also changes how
values are copied and how Rust lays out the enums built around `Value` (a slot is "unbound, a
value, or a shared cell"). The exact mechanism matters less than the measurement, which is
repeatable.

The fix is one attribute that says what was learned:

```rust
// 32 bytes, not the 24 the fields need: the VM indexes its stack and slots by value, and a
// power-of-two size makes that a shift. Measured: number loops ran ~9% slower at 24 bytes.
#[derive(Clone)]
#[repr(align(16))]
pub enum Value { … }
```

With it, the numeric loop runs in 0.94 s, slightly faster than before the whole change, and so
does creating closures (0.97 → 0.94 s).

## The comparison

Each row below is the best of three runs of each version. Runs of the two versions alternated on
the same machine, release build.

**Reading and building, all EELisp:**

| | before | after | |
|---|---|---|---|
| EELisp reader, the compiler's source (33K chars) | 23.8 ms | 19.4 ms | 1.22× |
| EELisp reader, eeditor's snippets (108K chars) | 64.2 ms | 52.1 ms | 1.23× |
| one bootstrap generation (read + compile 97 forms) | 34.0 ms | 29.6 ms | 1.15× |

**Compiled mode, per evaluation** (every form is read and compiled by EELisp first):

| | before | after | |
|---|---|---|---|
| `(+ 1 2)` | 21.9 µs | 18.6 µs | 1.18× |
| a small `defn` | 53.8 µs | 46.5 µs | 1.16× |
| load eeditor's snippet files | 89.6 ms | 75.8 ms | 1.18× |

**eeditor's snippet examples, run** (interpreted / compiled):

| | before | after | |
|---|---|---|---|
| zz-convert | 9.9 / 7.8 ms | 6.4 / 5.0 ms | 1.55× / 1.56× |
| zz-crypto | 20.5 / 16.0 ms | 14.3 / 10.9 ms | 1.43× / 1.47× |
| zz-fun | 1.1 / 1.1 ms | 0.79 / 0.72 ms | 1.40× / 1.52× |
| zz-reference | 3.1 / 2.8 ms | 2.3 / 2.2 ms | 1.35× / 1.27× |
| zz-text, zz-date, zz-math | | | 1.03–1.10× |

The synthetic workloads (`fib`, `loop`, `lists`, `let-heavy`, `closures`, `strings`) are 1.00–1.05×
faster in both engines.

Two things stand out:

- **The interpreter gains as much as compiled code.** This change isn't about the VM. It's about
  what every part of the engine pays to handle text. Real snippet code converts, formats and
  looks things up by name, so it handles a lot of text, and that's where the 1.3–1.5× comes from.
- **The reader gains less than the snippets do,** about 1.2×. With copies now cheap, what's left
  in the reader is the calls themselves: about 1.2 million of them for 108,000 characters,
  roughly 40 ns each. That's the VM's call path, the other item chapter 8 left open.

## The cost

`Value::Str` now holds an `Rc<str>` instead of a `String`, and `Value::Keyword` holds a `Sym`. That
changes the public type. Code that *makes* values mostly doesn't notice:
`Value::Str("text".into())` still compiles. Code that takes a string *out* and expects a `String`
needs `.to_string()`. eeditor's own code talks to the engine through JSON and `EngineHandle`, so it
doesn't change. The engine's own test files needed two small edits.

Because it's a public type change, it was first kept on its own branch, `exp/shared-strings`, for
review. It was then merged into `feat/bytecode-vm`.
