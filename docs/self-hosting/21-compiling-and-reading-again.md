# 21. Compiling and reading, again

*Branch: `exp/register-vm`. Tags: `register-vm-8` (where it starts), `register-vm-9`. Tools:
`examples/prof_compile.rs`, `examples/front_end.rs`, macOS `sample`.*

After chapter 20, loading eeditor's snippet files in compiled mode cost about 36 ms: 20 to read,
15.5 to compile, under 1 to assemble. Compiling had become as large as reading, and it had never
been profiled the way chapter 20 profiled the reader: summed per builtin, including everything
each builtin calls. This chapter does that, then takes the first step on the reader's shape.

## The compiler, by builtin

| `prof_compile`, 157 forms | share |
|---|---|
| the dispatch loop | 30% |
| dropping and copying values | 22% |
| `reduce`, inclusive | 10.5% |
| `list`, `map`, `dict-set`, `tail`, `append` | 2–3% each |

`reduce` was mostly `compiler-cat`, which joins instruction lists. It was
`(reduce append (list) parts)`: a call, and a fresh copy of the growing list, for every part.
`append` takes any number of lists, so it became `(apply append parts)`, one copy. Compiling went
from 14.5 to 14.25 ms.

The other idea was the compiler's boxes, its mutable counters and scope tables. Chapter 17 made
them cheaper closures, and they were still about 23,000 calls of an EELisp function per run. A
`box` builtin, a cell made in Rust, measured 14.25 → 14.1 ms: noise. A builtin closure reached
through `call-at` costs about what an EELisp one does. That wasn't worth adding a primitive to
the language, so it was reverted.

What's left in compiling is how it builds code. Every compiling function returns a list of
instructions, and its caller joins those lists into a bigger one, so an instruction is copied once
for every level of nesting above it. That's the 22% spent copying and dropping values. Building
into one list from the start, as the reader now builds its lists, would remove it, but every
compiling function would change. That's a rewrite, recorded here as the next one.

## The reader's shape, a first step

Every reading function returns a pair: the form, and the position after it. For each element of
a list, `reader-seq` called `reader-form`, which made such a pair, and then took it apart. For an
atom (a symbol, number, string or keyword, which is most tokens) the token already holds both the
value and the position after it. So `reader-seq` now turns an atom token into its form directly
(`reader-atom-form`), and only lists, quotes and dicts go through `reader-form`.

Reading the snippet files went from 20.0 to 19.5 ms. Making the pair was a small part of what a
token costs.

The larger part is the token itself, a list of four (kind, value, start, end), which every token
makes and drops. EELisp has no way to return more than one value without making a list. Removing
the token means either such a way (a language feature), or a position kept in shared mutable
state, which the reader has avoided so far. Both are bigger decisions than the steps in this
chapter.

## Where it stands

| eeditor's snippet files, compiled | chapter 15 | chapter 20 | now |
|---|---|---|---|
| read | 43.4 ms | 20.2 ms | 19.5 ms |
| compile | 14.9 ms | 15.5 ms | 14.9 ms |
| assemble | 0.9 ms | 0.8 ms | 0.8 ms |
| total | 65 ms | 36 ms | 35 ms |

From chapter 16 to here, getting code into compiled mode has become 1.85 times cheaper, and
running compiled code is up to twice as fast. The interpreter still loads the same files in
1.4 ms, because it reads with Rust and compiles nothing. Three larger moves remain, each a
decision rather than a step:

1. **Compiling into one list** instead of joining lists at every level. A rewrite of the
   compiler's code generation; the 22% it spends copying is the target.
2. **Returning more than one value** without making a list, which would let the reader drop
   its tokens and pairs. A language feature, so it would change EELisp itself.
3. **Word-sized values** (chapter 18), which make every instruction and copy cheaper,
   everywhere.
