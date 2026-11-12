# 15. The way in

*Branch: `exp/compiled-default`, on top of `exp/vm-redesign`. Tags: `vm-redesign-final` (where
it starts), `compiled-default-1`. Tools: `examples/front_end.rs`, `read_kinds.rs`,
`read_scale.rs`, `read_pieces.rs`, `startup.rs`, and the profiling harnesses `prof_one.rs` and
`prof_read.rs`.*

Chapter 14 left two weak spots and one big question. The weak spots: `map` and `filter` calling
small compiled functions were the one workload where compiled mode barely beat the interpreter,
and the reader slowed down badly on large files. The question: what stops compiled mode from
being the default?

## Builtins calling back into compiled code

When `map` calls a compiled function for each element, every call starts a fresh run of the VM.
The profile of `(map (fn (x) (* x 3)) …)` showed where that went:

- a fifth of the time was `malloc` and `free` of a vector holding one argument. `apply_value`
  copied its argument slice into a new `Vec` for every element;
- taking a spare machine from the thread's list and putting it back, which moved its 120 bytes
  each way, and cleared five vectors that a normal return leaves empty anyway;
- `enter` filled the new frame's slots with "unbound" and then overwrote the parameters.

Three commits, each measured on its own against the one before:

| change | map/filter | the rest |
|---|---|---|
| arguments borrowed from the slice, not collected (`vm::call_with`) | 1.13× | even |
| `enter` pushes arguments into their slots | 1.02× | 1.01–1.05× |
| spare machines boxed, cleared only after an error | 1.22× | even |

Against the end of chapter 14, `map/filter` went from 13.0 to 9.0 ms (1.44×), and calls in general
got 2–4%. The middle change is the one chapter 13 tried and measured as slower. Since then, `enter`
has become its own function that the dispatch loop never inlines, so changing its body no longer
moves the loop's layout. The same idea measured differently because the code around it had
changed.

## Reading in linear time

EELisp lists are vectors. `(append acc (list x))` copies `acc`, and `cons` copies its list too, so a
list built one element at a time costs n²/2 copies. The reader built three things that way: the
top-level forms of a text, the elements of each list, and strings with escapes (one `(str acc c)`
per character).

The fix is a *builder*: elements go into a short chunk (32), a full chunk joins a list of chunks,
and at the end a single `(apply append chunks)` joins everything once. A string with escapes is now
cut into the runs between escapes, joined once with `str-join`.

| `read-all`, compiled | before | after |
|---|---|---|
| 4,000 top-level forms | 168.3 ms | 64.1 ms |
| one list of 4,000 | 38.7 ms | 14.5 ms |

Doubling the input now doubles the time. `tests/reader.rs` gained texts sized around the chunk
boundary (31 to 65, 100, 1,000 elements) for lists, vectors, top-level forms and escaped
strings, so the builder's edge is checked against the Rust reader too.

## Where compiled mode's cost is

With those done, running compiled code beats the interpreter on everything measured: 1.12× on
`lists`, 2.1× on `fib` and loops, 1.1–1.6× on eeditor's snippets. What it loses is getting code in:

| | interpreter | compiled |
|---|---|---|
| start an engine | 0.16 ms | about 5 ms |
| `eval (+ 1 2)` | 0.34 µs | 16.8 µs |
| load eeditor's snippet files | 1.4 ms | 64.9 ms |

`examples/front_end.rs` splits a compiled evaluation into its stages. For the snippet files
(108 KB):

| read | compile | assemble | run |
|---|---|---|---|
| 43 ms | 15 ms | 0.9 ms | small |

Startup is mostly parsing the 124 KB image text (2.4 ms) and recompiling the prelude (about 1.5
ms). An editor doesn't feel 5 ms. Reading is what it would feel.

### A wrong guess

The reader scans a list of one-character strings. A string is a pointer with a reference count, so
copying one touches memory and comparing two compares bytes. A number does neither. So the
obvious idea was to scan code points: two builtins, `str-codes` and `str-from-codes`, and
128-entry tables instead of dict lookups.

It measured 46.3 → 44.7 ms, inside the noise. It was committed with its numbers and reverted. The
assumption was wrong: copying a one-character string costs one increment, and comparing it one
byte.

### Measuring instead

`examples/read_kinds.rs` reads one kind of text at a time:

| | per character | per token |
|---|---|---|
| comment lines | 99 ns | |
| whitespace | 175 ns | |
| strings | 302 ns | 3.5 µs |
| symbols | 450 ns | 3.8 µs |
| numbers | 719 ns | 3.2 µs |

The Rust reader takes about 7 ns per character. Whitespace is the plainest case: per character, a
bounds check, an `nth`, a call to `reader-class`, which does a dict lookup, and a comparison. That's
175 ns. Nothing in it is expensive on its own; it's the cost of running that much EELisp on this
VM. A token adds 2–4 µs: the atom's text, the token list, the parser's dispatch on its kind, the
builder.

One change follows from that directly. `reader-class` runs for every character, so it became a
macro, written out in each scanning loop instead of called. Whitespace went 175 → 130 ns per
character, symbols 450 → 398, and the snippet files 46.3 → 43.4 ms.

That change found a limit of chapter 10's procedure. The committed image couldn't build the new
source: it compiles the whole source before running any of it, so a macro defined and *used* in
the same source isn't known when the forms using it are compiled. `--from-source` built it, and
the new image then rebuilt itself identically. Chapter 10's test of a new macro passed because
nothing in the source used it.

## What it says

The reader is now linear, and small changes to it give 5–15%. To make compiled mode's front end
competitive with the interpreter's, it would have to get about thirty times faster, and the
per-character cost of EELisp code on this VM is the floor. Three ways forward:

1. **A faster machine.** The register VM and word-sized values from chapter 14 cut the cost of
   every instruction the reader runs. This is the self-hosting answer, and the slow one.
2. **Rust reads in compiled mode for now.** The EELisp reader stays in the image, in stage 2 of
   the bootstrap and in the tests that hold it to the Rust reader. The engine would use the Rust
   reader until the VM is fast enough. Loading the snippets would take about 20 ms instead of 65: what reading costs now (43 ms) gone, and about 1 ms of Rust reading in its place.
3. **Read each text only once.** Reading is pure: the same text gives the same forms. An editor
   reloads mostly unchanged files, so a cache keyed by the text would make re-reading free.
   Compiling isn't pure (macros and definitions change what it produces), so it can't be cached
   the same way.

Switching compiled mode on by default also means accepting, or fixing, the five known differences
from the interpreter listed in the roadmap. Each is about errors or about definition order, and
none changes a correct program's result.

## The first answer: read each text once

The third option came first, since it is safe today. In compiled mode the engine now remembers
what the EELisp reader made of each text it read, keyed by the text itself. The cache holds at
most 4 MB of source and starts over when it fills.

Only reading is remembered. A cached text's forms are still compiled and run every time, because
compiling depends on what is defined: a macro redefined since changes what a form compiles to.
`tests/compiled.rs` checks exactly that. The same text is evaluated before and after its macro is
redefined, and its definitions run again. A text that fails to read fails every time, since
errors aren't cached, and `(source f)` still finds a definition's text and comments when the
definition was read from the cache.

| compiled | first time | again |
|---|---|---|
| load eeditor's snippet files | 60.2 ms | 16.0 ms |
| `eval (+ 1 2)` | 18 µs | 5.4 µs |
| `eval` a `let` with a `map` | 87 µs | 29 µs |

An editor that reloads its files, or evaluates the same text repeatedly, now pays mostly for
compiling. The first load still pays for reading. That cost is the faster VM's job, the next
branch.
