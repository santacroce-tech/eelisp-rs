# 2. A virtual machine

*Code: `src/vm.rs`. Tests: `tests/vm.rs`.*

## What the interpreter does, and what a VM does instead

The tree-walking interpreter in `src/eval.rs` looks at a form, decides what it is, and acts:
a symbol is looked up, `(if c a b)` evaluates `c` and then one branch, a call evaluates its
arguments and applies the function. It does that every time the form runs. A loop that runs a
million times decides a million times that `(if …)` is an `if`.

A compiler makes those decisions once. It turns the form into a flat list of simple
**instructions**, and a **virtual machine** runs them. Ours is a *stack machine*: instructions
take their inputs from a stack of values and push their result back.

Here is `(+ a b)` as instructions:

```lisp
(load +) (load a) (load b) (call 2)
```

Push the value of `+`, then of `a`, then of `b`; then call the function that sits below two
arguments. When it returns, its result is on the stack where the function was.

## Bytecode you can read

The VM's input is ordinary EELisp data. A function body is a `proto`:

```lisp
(proto :name "add" :params (a b) :code ((load +) (load a) (load b) (tail-call 2)))
```

`vm-assemble` turns one into a function, and `vm-run` runs one at the top level:

```lisp
((vm-assemble '(proto :params (a b) :code ((load +) (load a) (load b) (tail-call 2)))) 2 3)
; → 5
```

This choice matters later. Because bytecode is EELisp data, a compiler written in EELisp can
produce it with `list` and `cons`, and a compiled program can be saved as text and read back
with the same reader that reads source code. There's no binary format to design.

## The instructions

| Instruction | What it does |
|---|---|
| `(const v)` | push `v` |
| `(load s)` | push the value of the variable `s` |
| `(def s)`, `(set s)` | bind or assign `s` to the value on top (and keep it there) |
| `(pop)` | drop the top value |
| `(jump L)`, `(jump-if-false L)` | jump to label `L`; the conditional form pops its test |
| `(jump-if-false-or-pop L)`, `(jump-if-true-or-pop L)` | for `and` / `or`: keep the value and jump, or pop it and go on |
| `(push-scope)`, `(pop-scope)` | enter or leave a child environment (for `let`) |
| `(closure (proto …))` | push a function that closes over the current environment |
| `(call n)`, `(tail-call n)`, `(return)` | calls, and leaving a function |
| `(dict k …)` | build a dict literal from one value per key |
| `(make-list item splice …)` | build a list, splicing the marked elements in (for quasiquote) |
| `(next-item L)` | step through a list (for `for-each`) |
| `(loop s …)`, `(recur n)`, `(end-loop)` | `loop` / `recur` |
| `(try L)`, `(end-try)` | `try` / `catch` |
| `(macro (proto …))` | push a macro whose expander is compiled |
| `(interp form)` | ask the interpreter to evaluate one form (the compiler's escape hatch) |
| `(label L)` | a jump target; not an instruction |

The assembler resolves labels, so a compiler never computes jump offsets. It also refuses
malformed bytecode with a clear message (`unknown instruction frobnicate`, `no label nowhere`)
instead of crashing at run time.

## One kind of function

The most important design decision is about what a compiled function *is*. It would have been
natural to add a new kind of value, a "compiled closure". We didn't. A compiled function is the
same `Function` the interpreter makes, with one more field:

```rust
pub struct Function {
    pub name: Option<String>,
    pub params: Vec<Sym>,
    pub rest: Option<Sym>,
    pub body: Vec<Value>,          // the source forms, for printing and (source f)
    pub closure: Env,
    pub code: Option<Rc<Proto>>,   // set when compiled
}
```

Everything that calls a function goes through two places, the evaluator and `apply_value`, and
both now check `code` first. So a compiled function works in `map`, `filter`, `reduce`,
`apply`, from a host's `Interpreter::call`, and from interpreted code. Interpreted functions
work from compiled code too. The two worlds mix freely, which is what lets the compiler be built
one form at a time.

## Variables: the same environments

Compiled code looks variables up in the very same environments the interpreter uses: chains of
scopes, searched by name. That is slower than it could be (a real compiler resolves a local
variable to a numbered slot) but it means compiled code *can't* disagree with interpreted code
about scoping. `def` inside a function defines in the function's scope; builtins receive the
caller's environment; closures share scopes exactly as before. Faster variable access is an
optimization the compiler can adopt later, with the interpreter's behavior already pinned down
by tests.

## Tail calls

EELisp promises that a call in tail position doesn't grow the stack: a function that calls
itself as its last act can do so a million times. The interpreter keeps that promise with a
trampoline. The VM keeps it more directly: `tail-call` **replaces** the current frame instead
of pushing a new one.

The VM's frames live on a heap-allocated stack, not on Rust's, so deep *non*-tail recursion
doesn't crash the process either. After 100,000 nested calls it stops with an error:

```
stack overflow: more than 100000 nested calls
```

Both behaviors are tested: `tail_calls_run_in_constant_space` counts down from a million, and
`deep_non_tail_recursion_is_an_error_not_a_crash` recurses 200,000 deep.

## `loop` and `recur`

In the interpreter, `recur` is a signal. It travels up until it reaches the innermost `loop`,
which rebinds its names and runs its body again. That signal can even cross function calls: a
function that calls `recur` restarts the loop that called it.

The VM mirrors this exactly. `(loop i acc)` records a restart point: where the body starts, which
environment holds `i` and `acc`, how high the stack was. `(recur 2)` takes two values off the
stack and raises the same signal the interpreter uses. The VM catches it at the innermost restart
point (in this frame or a caller's), rebinds the names, resets the stack and jumps back. Because
it is the *same* signal, a `recur` from an interpreted function restarts a compiled loop, and
the test `a_recur_from_an_interpreted_function_restarts_the_loop` says so.

## `try` and `catch`

`(try L)` records a handler: where to go, and the environment and stack height to go back to.
When an error happens, inside this function or in anything it calls, the VM unwinds to the
innermost handler, restores that state, pushes the error as a dict and jumps to `L`. The error
dict is built by one shared function, `eval::error_dict`, so both evaluators produce
`{:message … :kind …}` identically (`the_error_dict_matches_the_interpreter`).

Two details come straight from the interpreter's behavior:

- `try` doesn't catch `recur`. A `recur` inside a `try` restarts its loop, it isn't an error.
- A tail call inside a `try` (or a `loop`) becomes an ordinary call, so the handler or the
  restart point isn't thrown away with the frame.

## What the machine doesn't do

The VM never expands macros: if a macro reaches it at run time, that's an error, because
expanding macros is the compiler's job. It also leaves the choice of which arguments to evaluate
to the compiler. Builtins like `deftable` and `query` take some arguments unevaluated, and the
compiler has to know that. Keeping the machine small and the knowledge in the compiler is the
point: the compiler is the part written in EELisp.

Next: the compiler.
