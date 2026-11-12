# 20. The reader's own costs

*Branch: `exp/register-vm`. Tags: `register-vm-7` (where it starts), `register-vm-8`. Tools:
`examples/front_end.rs`, `examples/read_kinds.rs`, `examples/count_calls.rs`, and
`examples/prof_read.rs` with macOS `sample`.*

After chapter 19 a token cost about 1.7 µs, and the counts pointed at the reader's parser: a
quarter of its calls were `=`, comparing each token's kind against `:eof`, `:close`, `:open` and
the rest, one after another. That was the plan. It turned out to be the wrong place, and finding
the right one took two profiles, not one guess.

## The comparisons

Three changes, each removing comparisons:

- `reader-token` decides what a token is from its first character with one table (the
  punctuation, plus comma and the string quote); anything not in it starts an atom.
- `reader-symbol` checks `true`, `false` and `nil` with one `dict-has`, not three `=`.
- `reader-skip` tests for `;` directly, and any other character in the class table starts a
  token.

`=` calls fell from 153,000 to 118,000, and reading the snippet files went from 25.7–26.9 ms to
26.1 ms: no change. Every one of those comparisons was already an intrinsic inside `call2`
(chapter 17), and a cheap one. Comment lines did get faster (50 → 33 ns per character), because
`reader-skip` now goes straight past a comment, so the changes were kept.

One mistake was caught on the way, by the test that holds the reader to the Rust one. After a
comment the new loop landed on the newline without scanning past it, and since newlines are in
the class table it took one for the start of a token. Every position the loop moves to has to
come from a scan.

## A profile by builtin

The sampling profiler, summed per builtin including everything it calls, gave a different
picture. The largest share was `map`, which was `read-top` building each form's source text and
finding the comment block above it. `reader-line-start` walked backwards one character per loop
turn, once for every line of a doc comment. A third scanning builtin, the mirror image of the
other two, fixed that:

- `(scan-back-until lst i table)`: the position just after the nearest element before `i` that
  is in the table, or 0. With a table of newlines, where a line starts.

The trailing-comment and blank-line checks moved to `scan-while` as well. Reading went from 26.1
to 24.6 ms.

## Two builtins that allocated too much

The same profile showed `str-join` and `str-chars` near the top, and both turned out to allocate
much more than they needed to.

**`str` and `str-join`** made a string for each element, through the printer, and then joined
them. A token's text is `(str-join "" characters)`, so every character cost an allocation. They
now append each string element as it is (anything else still goes through the printer), and
allocate once for the result. The output is the same for every kind of element.

**`str-chars`** made a new one-character string for every character: 108,000 allocations per
read of the snippet files, and as many frees when the list died. Strings never change, so it now
hands out one shared string per ASCII character, made once.

| the snippet files | before | after |
|---|---|---|
| `str`/`str-join` append in place | 24.6 ms | 21.9 ms |
| `str-chars` shares ASCII characters | 21.9 ms | 20.2 ms |

Both are general builtins, so programs gain too: eeditor's `strings` snippet runs 1.08× faster
interpreted and 1.20× faster compiled, and nothing else moved.

## Fx, a second time

With `scan-while` now doing a dict lookup per character, SipHash had grown to 6% of reading, so
chapter 16's cheaper hash was tried again. Whitespace and comment lines got faster (26 → 12–18
and 33 → 20 ns per character), but the snippet files read in 20.0 ms against 20.2: even. On real
files it gains nothing, and it would give up SipHash's resistance to keys chosen to collide,
which matters for dicts that hold data from outside (JSON from a host, or over HTTP). Reverted
again, with both sets of numbers in the history.

## Where reading stands

| the snippet files, compiled | chapter 15 | chapter 19 | now |
|---|---|---|---|
| read | 43.4 ms | 27.6 ms | 20.2 ms |
| compile | 14.9 ms | 15.5 ms | 15.5 ms |
| assemble | 0.9 ms | 0.8 ms | 0.8 ms |

Reading is 2.1 times faster than at the start of chapter 16. Loading eeditor's snippet files in
compiled mode now costs about 36 ms all told, against 65 ms then (and against 1.4 ms in the
interpreter, which reads with Rust and compiles nothing).

The profile now:

| | share |
|---|---|
| the dispatch loop | 32% |
| dropping and copying values, allocating, freeing | about 26% |
| hashing (dict lookups) | 6% |
| the scans | 7% |

What remains is the reader's own shape. Every token is a list of four, and every reading function
returns a pair (the form and the next position), so each token costs several small lists that
are made and dropped at once. Removing them means changing how the reader's functions pass
results to each other, not adding builtins. And the dispatch share is the number of instructions
the reader's EELisp runs per token, which only that rewrite, or word-sized values (chapter 18),
would change.
