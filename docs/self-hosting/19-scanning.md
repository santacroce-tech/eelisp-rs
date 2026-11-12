# 19. Scanning

*Branch: `exp/register-vm`. Tags: `register-vm-6` (where it starts), `register-vm-7`. Tools:
`examples/front_end.rs`, `examples/read_kinds.rs`, `examples/count_calls.rs`.*

Chapter 18 ended on a decision. Reading was no longer limited by how the machine makes a call,
but by the reader's own design: about ten instructions per character, and three new lists for
every element added to a list being built. Three ways forward were put on the table. The choice
was to do the first two, in order, each measured on its own: primitives that scan a run of
characters in one call, then a way to build lists without copying them. Word-sized values wait.

## What stays in EELisp

The reader was written in EELisp for a reason (chapter 9): text to bytecode with no Rust between
them. Moving the reader's inner loops into Rust changes that, so the line was drawn carefully.
Every decision stays in `src/reader.eel`: what a token is, which characters make a run, how forms
nest, what each kind of token becomes, every error and its message. What moves is the loop that
asks "is the next character one of these?" over and over. That loop now runs as two general
builtins, written so that they know nothing about reading:

- `(scan-while lst i table)`: from position `i`, the first position whose element isn't in the
  dict `table`;
- `(scan-until lst i table)`: the first position whose element is.

Both return the list's length when the run reaches the end. Readers written in Lisp have long
leaned on their host this way, through `peek-char`, `read-line` or a pattern matcher.

## Scanning

The lexer scans four kinds of run:

| run | table | was |
|---|---|---|
| whitespace | `reader-spaces` (space, newline, tab, return) | one loop turn per character |
| a comment to the end of its line | `reader-newline`, with `scan-until` | one loop turn per character |
| an atom's characters | `reader-words` | one loop turn per character |
| a string's plain text | `reader-string-stops` (`"` and `\`), with `scan-until` | one loop turn per character |

A character in none of the tables stops a scan: an accented letter, another alphabet, a rarer
kind of space. The reader then decides it as it did before, through `reader-class` (Rust's own
definition of whitespace, via `str-trim`), and scans on. So nothing about what the reader accepts
changed. `tests/reader.rs` holds it to the Rust reader on the repository's own files, a list of
edge cases and random strings over the characters that matter, all unchanged, and it agrees.

| | before | after |
|---|---|---|
| read the snippet files | 35.1 ms | 30.5 ms |
| whitespace, per character | 100 ns | 29 ns |
| comments | 95 ns | 50 ns |
| symbols | 316 ns | 265 ns |
| strings | 204 ns | 160 ns |

Whitespace is now four times cheaper and approaching the Rust reader's 7 ns. What remained was
the cost of each token, about 2 µs.

## Building lists

Every list the reader reads, and the list of top-level forms, was built with chapter 15's
builder, a pair of a list of full chunks and the current chunk. Adding one element made three new
lists: the element in a list, the chunk with it appended, and the pair. That kept building
linear, but allocated on every element.

`(list-builder a …)` makes a builder: a builtin function holding a vector of its own. Calling it
with values adds them, in place. Calling it with none hands back the list made so far and leaves
the builder empty. The list it hands back is never touched again, so no EELisp list is ever seen
to change; only the builder does. In the reader each builder belongs to one loop, which adds one
element after another and never looks back, so `reader-add` simply returns the builder it was
given.

| | before | after |
|---|---|---|
| read the snippet files | 30.5 ms | 27.6 ms |
| per token: symbols | 2.24 µs | 1.81 µs |
| numbers | 2.03 µs | 1.69 µs |
| strings | 1.84 µs | 1.58 µs |

## Where reading stands

| the snippet files, compiled | chapter 15 | chapter 18 | now |
|---|---|---|---|
| read | 43.4 ms | 35.1 ms | 27.6 ms |
| calls while reading | | 1.14 million | 620,000 |

Reading is 1.6 times faster than at the start of chapter 16, and the reader makes half the calls
it did at the start of this chapter. The largest share left is `=` (a quarter of the calls). The
parser compares each token's kind against `:eof`, `:close`, `:open` and the rest in `cond`
chains, and the remaining per-token cost is that dispatch, plus the token itself (a list of four),
its text (`slice` and `str-join`), and classifying an atom as a keyword, number or symbol. Those
are changes to the reader's EELisp, not to the machine or the builtins, and the next measurements
would start there.
