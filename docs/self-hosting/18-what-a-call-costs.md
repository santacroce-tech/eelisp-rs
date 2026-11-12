# 18. What a call costs

*Branch: `exp/register-vm`. Tags: `register-vm-5` (where it starts), `register-vm-6`. Tools:
`scripts/ab.sh`, `examples/front_end.rs`, `examples/prof_read.rs` with macOS `sample`.*

Chapter 17 ended with the reader making about ten calls per character, nearly all of them
register instructions or intrinsics by then. What was left looked like the price of a call itself.
Every profile since chapter 16 had the same two lines near the top, copying values
(`Value::clone`) and dropping them, together about a quarter of the time. And register calls had
a cost of their own, the slots a callee shares with its caller. This chapter tests both.

## The slots a register call shares

A compiled function entered at its caller's argument registers shares the caller's region of the
slot vector. Until now, entering one cut the vector back to the arguments and pushed the callee's
other slots; returning cut it back to the callee's start and refilled the caller's region with
unbound slots, so the caller found all its registers again.

Chapter 17 made every register write a new binding (as `init-local`), and that makes the
refilling unnecessary. A caller never reads a temporary it hasn't written since, and writing one
replaces whatever is there, a value the callee left or a cell, without writing through it. So
entering now makes only the callee's own slots after its parameters unbound. Returning removes
only what lies beyond the caller's region. An error that unwinds to a caller's `try` still
restores the caller's registers explicitly, as before.

| `scripts/ab.sh` | |
|---|---|
| `fib 25` | 1.06× |
| closures 200k | 1.04× |
| calls 300k | 1.03× |
| tail calls, loop, map/filter | 0.98–1.00× |

Small, and kept. Reading didn't move, since its time isn't in calls between compiled functions.

## Copying values inline

`Value` is an enum of 19 kinds. Its `Clone` is derived: one large match that the compiler
doesn't inline. So every copy of a string, list or function is a call to it. The VM already
copied numbers, booleans and nil itself. The experiment added the common reference-counted
kinds: strings, symbols, keywords, lists, dicts, functions and builtins, each copied by bumping
its count, inline, in the VM and in the intrinsics.

`fib` 1.05×, `loop 1M` 0.97×, everything else 1.00–1.03×, and reading 35.1 ms either way. That's
noise; it was reverted. The `clone` samples in the profiles were never the call around the copy.
They were the copy itself: a reference count touched in memory, wherever the value lives.

## Where reading's time is now

A fresh profile of the reader on eeditor's snippet files, at the machine level:

| | share |
|---|---|
| the dispatch loop | 40% |
| allocating and freeing | 22% |
| calls, copies, drops, builtins | the rest |

The freeing is mostly in `ret`: values a function made, dying when it returns. Most are made by
the reader's own algorithms. EELisp lists are vectors and never change, so `reader-add`, which
adds one element to a list being built, makes three new lists each time: the one-element list,
the chunk with it appended, and the pair holding the builder. Token text is a list of
one-character strings, joined by `str-join`.

That's the finding of this chapter. The machine's per-call costs are no longer where reading
spends its time. The two left are the number of instructions the reader runs per character
(about ten calls' worth, each a dispatch), and what the reader allocates. Neither is about how
the VM makes a call.

## What would change it

Three ways forward, each changing what the reader is rather than how it runs:

1. **Scanning primitives.** A builtin such as `(str-span text i class-table)` returns where a run
   of characters of one class ends, so whitespace, a symbol's characters or a string's body
   cost one call each, not ten instructions per character. The reader's logic (what a token is,
   how forms nest, what each kind becomes) stays in EELisp; only the inner loop of a scan moves
   to Rust. This is how readers written in Lisp often lean on their host, and it's the change
   that could bring reading near the Rust reader's speed. It does move work out of EELisp,
   which is a decision about what "the engine reads in EELisp" should mean.
2. **A cheaper way to build lists.** A builder primitive that appends in place (owned by one
   function, so immutability is never observable) would remove the three allocations per
   element. It touches the language's promise that lists don't change, but only inside a
   builtin.
3. **Word-sized values.** Chapter 14's NaN-boxing: a `Value` in 8 bytes, numbers and small
   immediates with no reference count. It changes the cost of every instruction and every copy,
   and every line of Rust that looks inside a `Value`. It's the largest change, and the only one
   of the three that speeds up everything, not just reading.

| the snippet files | chapter 15 | chapter 17 | now |
|---|---|---|---|
| read | 43.4 ms | 35.3 ms | 35.1 ms |
| compile | 14.9 ms | 15.1 ms | 15.8 ms (`front_end`), 15.1 ms (`prof_compile`) |
