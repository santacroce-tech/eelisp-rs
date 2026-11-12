# 22. Compiling into one list

*Branch: `exp/register-vm`. Tags: `register-vm-9` (where it starts), `register-vm-10`. Tools:
`examples/prof_compile.rs`, `examples/front_end.rs`, `examples/count_calls.rs`.*

Chapter 21 found where the compiler's remaining time goes. Every compiling function returned a
list of instructions, and its caller joined those lists into a bigger one: the code for `(+ a b)`
was a list, the `if` around it copied it into its own, the `let` around that copied it again, and
so on up to the function's body. An instruction was copied once for every form around it, and
copying and dropping values was 22% of compiling.

## One list per function

The fix is the one chapter 19 used for the reader's lists. Each function's code is built in one
`list-builder`, and every compiling function takes it as `out` and adds its instructions there,
in order, instead of returning them:

```lisp
(defn compiler-if (x cx tailp out)
  (let (else-l (compiler-label) end-l (compiler-label))
    (compiler-jump-unless (nth x 1) cx else-l out)
    (compiler-expr (nth x 2) cx tailp out)
    (out (list 'jump end-l) (list 'label else-l))
    (if (> (length x) 3) (compiler-expr (nth x 3) cx tailp out) (out (list 'const nil)))
    (out (list 'label end-l))))
```

An instruction is now made once and never copied by the forms around it. A function's proto
gets its own builder, and so does `compile`'s top-level form.

One kind of form doesn't fit the order: a block whose first instructions depend on its end. A
`let`, a `loop`, a `catch` handler and a `for-each` body start by clearing the slots their `def`s
use, so that each run of the block starts with them unbound (chapter 6). Which slots those are is
known only once the body has been compiled. Such a block builds its code in a builder of its own,
and then adds the clears and that code to `out`: one copy, at that level only.

The builder has one trap. `(out)` with no arguments hands back the finished list and empties the
builder, so instructions always go in one or more at a time. A list made separately is added only
when it isn't empty (`compiler-emit-all`).

## Checking it

It is a rewrite of every compiling function, so it was checked three ways:

- **The differential test** (517 snippets against the interpreter, four setups) passed
  unchanged.
- **The crossing.** The committed image built the new compiler, generations 2 and 3 were
  identical, and the interpreter made the same image.
- **The code itself.** The image holds the reader too, compiled from source that didn't change.
  Comparing its 31 reader protos between the old image and the new one, the only differences are
  label numbers: the compiler, compiled before the reader, now uses a different number of labels.
  With labels renumbered, the code is identical. So programs run exactly the code they ran before;
  only producing it got cheaper.

| compiling eeditor's snippet files | before | after |
|---|---|---|
| `prof_compile` | 14.5 ms | 12.5 ms |
| `front_end` | 14.9 ms | 12.8 ms |
| calls per form | 2,434 | 2,268 |

## Where it stands

| eeditor's snippet files, compiled | chapter 15 | now |
|---|---|---|
| read | 43.4 ms | 19.0 ms |
| compile | 14.9 ms | 12.8 ms |
| assemble | 0.9 ms | 0.8 ms |
| total | 65 ms | 33 ms |

Getting code into compiled mode is now half what it cost at the start of chapter 16, and compiled
code runs up to twice as fast. Two large moves are still open (chapter 21): returning more than
one value without making a list, which would let the reader drop its token lists and result
pairs, and word-sized values.
