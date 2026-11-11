# 13. A cheaper call

*Code: `src/vm.rs` (`Vm::call`, `load_global`), `src/env.rs` (`layout`), `FastOp` in
`src/value.rs`. Tests: `tests/compiled.rs`.*

## Where a call's time goes

Chapter 8 ended by pointing at the VM itself: every call pays the full cost of the machinery. The
clearest program to study that with is `fib`, nothing but calls and arithmetic:

```lisp
(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
```

Compiled, one call of `fib` cost about 100 ns, the same as the tree-walking interpreter. Each call
runs about twenty instructions: five global lookups (`<`, `+`, `-`, `fib`, `fib`), four builtin
calls, two calls of `fib` itself, and the jumps and loads in between. A sampling profile split the
time like this:

| | |
|---|---|
| dispatching instructions (`Vm::step`) | 28% |
| the call machinery: `call`, `enter`, `ret`, frames and slots | about 30% |
| the arithmetic builtins, through their generic code | about 17% |
| dropping values | 11% |
| looking up globals | 6% |

## Measuring small changes

The first attempt was to make the call machinery tidier: write the arguments straight into the
new frame's slots, and stop collecting a tail call's arguments into a temporary vector. Measured
against the previous build, plain calls got **5–9% slower**.

So before going further, the measurement itself needed checking. Running the old binary against
an identical copy of itself, alternating runs, gave 1.000×. The method was sound; the slowdown was
real. In an interpreter's dispatch loop, a small edit can change how the compiler lays out the
whole hot function, and that moves the timing by a few percent either way. The tail-call change
stayed, since it removes an allocation per tail call. The rewrite of `enter` went. So did a later
idea, dropping numbers inline instead of through the generic drop code, which measured 8% slower.

The lesson shaped the rest of the chapter: look for costs big enough to stand out from layout
noise, measure every change against the previous build, and keep only what measures better.

## Arithmetic without a call

`(+ a b)` was an ordinary call. Look up the global `+`, push the builtin, push the two arguments,
call through a function pointer into the generic `add`, which loops over its arguments, checks
that each is a number, sums, and wraps the result.

Now the language's own arithmetic and comparison builtins carry a tag:

```rust
pub struct Builtin {
    …
    /// Set only on the language's own arithmetic and comparisons.
    pub fast: Option<FastOp>,      // Add, Sub, Mul, Lt, Gt, Le, Ge, Eq, Ne
}
```

When the VM calls a builtin with two arguments, it checks the tag. If both arguments are numbers,
it computes the result in place. Anything else (three arguments, a string, a NaN) goes through the
builtin as before, so the results and the errors stay exactly the builtin's own.

The tag is on the *value*, not on the name. A program that defines its own `*` binds `*` to its
own function, which has no tag, so the fast path doesn't apply and the program's `*` is what runs.
A test checks exactly that.

This was the first clear win: `fib` 1.14×, a loop of calls 1.15×, tail calls 1.26×.

## Remembering where a global lives

Every global variable read was a hash-table lookup by name. Chapter 8 tried caching those and gave
up. Its cache was invalidated by a counter bumped on every global *write*, and the compiler writes
its label counter constantly, so the cache was always cold.

The fix is to be precise about what can actually invalidate the cache. The cache remembers *where*
a global's value lives in memory. In a hash map, an entry moves only when the map gains or loses
a key, because only then can it grow and rearrange. Changing an existing entry's value leaves it
where it is. So:

- `env.rs` keeps a single counter, `layout`, bumped only when a global scope is created or gains a
  new name. `set!`, and redefining a function that already exists, update the value in place and
  leave the counter alone.
- Each compiled function has, for each global name it uses, a small cache entry: which global
  scope the name was found in, the `layout` at that moment, and the address of the value.
- `Load` checks that the frame's scope is that same global scope and that `layout` hasn't moved.
  If so, it reads the value at that address. Otherwise it looks the name up as before and refills
  the cache.

```rust
fn load_global(env: &Env, proto: &Proto, s: u32) -> Result<Value, LispError> {
    let c = proto.globals[s as usize].get();
    if c.scope == Rc::as_ptr(env) && c.layout == env::layout() {
        if unsafe { env.try_borrow_unguarded() }.is_ok() {   // nobody is changing it
            return Ok(copy(unsafe { &*c.value }));
        }
    }
    … // look it up by name, and remember where it was found
}
```

This is the one place in the VM that reads through a raw pointer, so its safety argument is
written down next to it, and the tests try to break it. They redefine a cached function, `set!` and
`def` a cached variable, add two thousand new globals so the map has to grow and move every entry,
and run two engines side by side, each with its own globals. A new global scope also bumps the
counter. Without that, a scope created at the address of one just freed could match a stale cache
entry.

## The result

Best of three, alternating with the build before this chapter, release:

| compiled | before | after | |
|---|---|---|---|
| `loop` | 72.8 ms | 54.1 ms | 1.35× |
| `let-heavy` | 60.2 ms | 47.4 ms | 1.27× |
| `fib` | 11.1 ms | 9.1 ms | 1.22× |
| `closures` | 58.9 ms | 53.1 ms | 1.11× |
| `strings` | 38.4 ms | 34.2 ms | 1.12× |
| eeditor's snippet examples | | | 1.00–1.15× |
| the EELisp reader | 52.3 ms | 48.2 ms | 1.08× |

The interpreter is unchanged, as it should be, since none of this touches it. So the gap between
the two engines widened. Compiled `loop` is now 1.9× the interpreter's speed, and `fib` 1.3×.

What's left in the profile is the dispatch loop itself (about half the time) and the frame a call
builds. Going further means a different shape of VM: fewer, bigger instructions, or a frame that
isn't built field by field on every call. Those are larger redesigns, worth measuring before
committing to one.

One limit turned up along the way. A test that put three thousand definitions in a single string
took over a minute in a debug build. The reader collects top-level forms with `append`, and since
EELisp lists are vectors, that's quadratic. Files with a few hundred forms don't notice. A file with
thousands would.
