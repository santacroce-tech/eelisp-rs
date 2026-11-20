# 1. Why self-host, and how

## The question

EELisp started as a Swift engine and was rewritten in Rust. Either way, the language was
*defined by a program in another language*. If you wanted to know what `(loop …)` means, the
final answer was a few hundred lines of Rust in `src/eval.rs`.

A **self-hosted** language turns that around. Its compiler is written in the language itself and
compiles its own source. Once that works, the language describes itself: the compiler is an
EELisp program, and so is the definition of every special form it compiles.

That was the goal from the start, and it's worth being precise about what it is and isn't:

- It **is**: an EELisp compiler, written in EELisp, that compiles itself.
- It **is not**: getting rid of Rust. Something has to run the compiled code. Erlang's compiler
  is written in Erlang and runs on a virtual machine written in C; `javac` is written in Java and
  runs on a JVM written in C++. Nobody says those languages aren't self-hosted.

## The options

We looked at four shapes before writing any code.

| Option | What it means | Verdict |
|---|---|---|
| Build on Chez Scheme | Translate EELisp to Scheme and let Chez compile it | Fast, mature, proper tail calls for free. But Scheme's semantics (a full numeric tower, mutable pairs, its own error system) fight EELisp's (one float type, immutable values, insertion-ordered dicts, errors as dicts), and Chez's native code generator makes the browser and WebAssembly targets hard. Most of the work would go into making Chez behave like EELisp. |
| Compile to C, with a C runtime | The compiler emits C; a hand-written C runtime provides values and a garbage collector | Portable everywhere, and free of Rust. But it gives up Rust's memory safety (a runtime bug crashes the host application instead of returning an error), and every host would need a new integration. |
| Compile to Rust source | The compiler emits Rust that links against today's runtime | Keeps safety. But EELisp code arrives at run time — someone types a snippet into an editor — and calling `rustc` for each one is far too slow. |
| **Compile to bytecode for a VM in Rust** | The compiler emits instructions for a small virtual machine written in Rust | **Chosen.** |

The bytecode option keeps everything that already works. The CLI, the `--serve` JSON protocol,
the thread-safe `EngineHandle`, the WebAssembly build and the editor that embeds the engine all
call `eval` and get a value back; they don't care whether that value came from walking a tree
or running instructions. The compiler lives inside the engine, so a snippet compiles in
milliseconds with no external tool. And the result is genuinely self-hosted.

## The plan

```
          compiler.eel  (reader, macro expansion, code generation — all EELisp)
                │  compiles itself
                ▼
          compiler image (bytecode)
                │
  snippet ──►  VM in Rust  ──► result
               (values, builtins, database, agenda, sheets: unchanged)
```

Five steps, each one useful on its own:

1. **A virtual machine in Rust**, tested with bytecode written by hand.
2. **A compiler in EELisp**, run by the existing interpreter, checked against it.
3. **The bootstrap**: the compiler compiles itself, and the output is stable.
4. **Switch `eval`** to compile-and-run, once tests and a benchmark agree.
5. **Move libraries into EELisp**, now that EELisp code can be compiled.

The old tree-walking interpreter stays throughout. It is the reference every step is tested
against, and it is how the compiler can always be rebuilt from source, so the language never
depends on a binary nobody can reproduce.

## Rules we kept

- **The test suite is the referee.** Every step keeps every existing test passing, and every
  new behavior has a test.
- **Same meaning, not similar meaning.** Compiled code must give the same results *and the same
  errors* as interpreted code. Where it can't yet, the difference is written down.
- **Measure before switching.** The VM replaces the interpreter only on evidence.

The next chapter builds the machine.
