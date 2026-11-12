# 23. Word-sized values

*Branch: `exp/register-vm`. Tags: `register-vm-10` (where it starts), `register-vm-11`. Tool:
`examples/nanbox_proto.rs`.*

Chapter 14 named two big changes that could make every instruction cheaper: a register machine
and word-sized values. Chapters 16 and 17 did the first. Chapter 18 put the second back on the
table, after copying and dropping values kept showing up as about a quarter of every profile. It's
also by far the largest change left: every line of Rust that looks inside a `Value` would change.
So, as with the register machine, it was prototyped and measured before anything else.

## The idea

A `Value` today is 32 bytes: a tag saying which kind it is, and a payload. A number is an `f64` in
that payload; a string, list, dict or function is a reference-counted pointer. Every register,
every stack slot and every list element is 32 bytes, and every copy moves all of them.

NaN-boxing, as LuaJIT and several JavaScript engines use it, makes a value 8 bytes. A
floating-point number has many bit patterns that all mean "not a number", and arithmetic only
ever produces one or two of them. The rest are free to carry other things. A number is its own
64 bits. Nil, true and false are three fixed patterns. A pointer to a string, list or dict is a
tag in the top 16 bits and the address in the low 48. Reading a value as a number means checking
it isn't one of the tags; copying one means copying a word, and bumping a count only if it's a
pointer.

## The prototype

`examples/nanbox_proto.rs` is one small register machine, written once and generic over its value
type. It runs the same three programs with the engine's own `Value` and with `Boxed`, the 8-byte
value. Everything else is shared: the registers, the frames, the arithmetic, `nth`, `dict-get`,
the reference counts. So whatever differs is the representation.

| | 32-byte `Value` | 8-byte `Boxed` | |
|---|---|---|---|
| `fib 25` | 5.3 ms | 4.8 ms | 1.07–1.14× |
| a loop of 1M | 11.5 ms | 13.1 ms | 0.88× |
| the reader's whitespace loop | 10.0 ms | 7.2 ms | 1.35–1.40× |

Getting there took three versions, and the numbers moved each time. The first measured
1.07 / 0.84 / 1.44×. It checked every arithmetic result for the tagged patterns, which is
needless: arithmetic on numbers never makes one, since the hardware's "not a number" is the
canonical pattern and one with its sign bit set still reads as a number here. Dropping that check
and simplifying the pointer test moved the loop to 0.76×, and even moved the 32-byte side, whose
code hadn't changed. Removing one unused instruction from the prototype settled it at the numbers
above, the same in three runs. A dispatch loop's layout again (chapters 14, 17), even in a
prototype a few hundred lines long.

## What it says

Word-sized values help where values are copied: the whitespace loop copies a string out of a list
and a number out of a dict, and gains 1.4×. They cost where values are only numbers: every read of
a number now has to rule out the tags first, and the counting loop loses 12%. Calls gain a
little. That's a mixed result, not the across-the-board gain that would justify rewriting every
`match` on a `Value` in the engine. It is set aside for now, with the prototype kept so the
question can be asked again on a different machine.

The prototype said something else, more useful. With the *same* 32-byte values, this small
machine runs the three programs **twice as fast as the real one**:

| | prototype, 32-byte values | `src/vm.rs` |
|---|---|---|
| `fib 25` | 5.3 ms | 11.6 ms |
| a loop of 1M | 11.5 ms | 24.4 ms |
| the whitespace loop | 10.0 ms | 18.8 ms |

So the real machine's own structure costs more than the size of its values. The prototype leaves
out much of what the real machine has to do: cells shared with closures, unbound slots meaning a
global, loop marks, `try` handlers, errors with their kinds. And one thing that stands out in
every arithmetic step: in EELisp a program may redefine `+` or `<`. So the real machine's
`(< n 2)` is a `call2` that checks, through its global cache, that `<` is still the builtin it
was (the scope, the layout of the globals, and that no one is changing them right now) before
computing anything. The prototype just has an `Lt` instruction. That check runs for every `+`,
`-`, `<` and `=` in every loop.

That is where the next measurement goes: what each of those differences costs, starting with the
cost of honouring redefinable arithmetic and how to make it cheap when no one has redefined it.
