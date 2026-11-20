# 9. A reader in EELisp

*Code: `src/reader.eel`, the `stage2` function in `src/boot.rs`, three new builtins in
`src/builtins.rs`. Tests: `tests/reader.rs`, and a new test in `tests/bootstrap.rs`.*

## The other half

A language implementation has two halves. The **reader** turns text into data. The
**compiler** (or an interpreter) turns that data into something that runs. Chapter 4 made the
second half self-hosted: the EELisp compiler compiles itself. But every time it did, Rust read
the source first. `parser::parse` turned `src/compiler.eel` into lists, and only then did EELisp
take over.

In a Lisp the reader matters more than usual, because it is the language's *entire* syntax.
There are no statements, no operator precedence, no grammar beyond "a list is things between
parentheses". Everything else (`defn`, `let`, `if`) is a list the compiler recognizes. So a
reader written in EELisp means the syntax is defined in EELisp too.

This chapter writes that reader and closes the loop:

```
            text ──read──▶ forms ──compile──▶ bytecode
before:          Rust            EELisp
now:             EELisp          EELisp
```

## What it needs from the host

A reader works on characters, so the first question is what EELisp offers for those. The answer
was: less than a reader needs. Three builtins were added, and each one is something no amount of
EELisp code could provide:

- **`(str-chars s)`**: a string's characters, as a list of one-character strings. A reader looks
  at position `i` over and over. `substr` can do that, but it counts from the start of the string
  every time, which makes reading a file quadratic. EELisp lists are vectors underneath, so
  `(nth cs i)` is a single step.
- **`(symbol "name")`** and **`(keyword "name")`**: a reader has to *make* symbols and keywords
  out of the text it reads. Until now only the Rust reader could create one. Clojure has the same
  two functions under the same names.

The fourth thing it needs already existed: `->number` turns `"1e3"` into 1000 using Rust's own
number parser. Reading a decimal number with correct rounding is a famously subtle algorithm, and
Scheme implementations written in Scheme still call C's `strtod` for it. Calling the host for it
is the same choice.

## Two layers

The reader is built like the Rust one. A **lexer** cuts characters into tokens: `(`, `'`, a
string, a number, a symbol. A **parser** builds forms out of tokens. The difference is that here
tokens are made on demand. The parser asks for "the token at position `i`" and gets back a small
list:

```lisp
(kind value start end)      ; e.g. (:sym "defn" 1 5), (:open "(" 0 1), (:eof nil 120 120)
```

Every reading function takes a position and returns the form it read with the position after
it:

```lisp
;; The elements of a list up to its closer.
(defn reader-seq (cs n i close acc)
  (loop (i i acc acc)
    (let (tok (reader-token cs n i))
      (cond
        (= (head tok) :eof) (reader-fail cs n (nth tok 3) "unclosed list")
        (and (= (head tok) :close) (= (nth tok 1) close)) (list acc (nth tok 3))
        true (let (r (reader-form tok cs n))
               (recur (nth r 1) (append acc (list (head r)))))))))
```

`[a b]` uses the same function, starting the list with the symbol `list`, because `[a b]` *is*
`(list a b)`. Quotes work the same way: `'x` reads as `(quote x)`, `` `x `` as `(quasiquote x)`.
At the top level, a stray `)` is skipped rather than fatal, as in the Rust reader, so one
unbalanced paren doesn't lose the rest of a file.

## Agreeing exactly

"Reads EELisp" isn't a precise enough goal. The goal is: **for any text, the same forms as the
Rust reader, or the same error message.** That's what `tests/reader.rs` checks, the same
differential testing as chapter 3, this time on three kinds of input:

- **The repository's own files**, 75 of them, about a megabyte: the compiler, the reader itself,
  the bootstrap image, but also every Rust test and every Markdown chapter. Rust and Markdown
  aren't EELisp, which is the point. They're full of apostrophes, unbalanced parentheses and
  half-finished strings, the kind of input a hand-written test never thinks of.
- **39 edge cases** written by hand.
- **1,500 random strings**, built from the characters that matter to a reader: brackets, quotes,
  backslashes, `;`, `0x`, `inf`, a non-breaking space… The random generator has a fixed seed, so
  a failure always repeats.

Each runs twice, on the reader interpreted and on the reader as bytecode from the image. Both
agree on everything.

Getting there meant learning what the Rust reader actually does. Nobody had written these rules
down, because nobody had needed to:

- **`nan`, `inf` and `infinity`** (in any case, with a sign) are numbers, because Rust's number
  parser accepts them. `(+ inf 1)` is infinity.
- **`0x+ff` is 255.** Hex literals are read by Rust's integer parser, which accepts a sign after
  the `0x`. So `-0x-ff` is 255 too.
- **Whitespace is Unicode whitespace.** A non-breaking space separates two symbols. The EELisp
  reader asks `str-trim`, which uses the same definition.
- **An unterminated string beats an earlier syntax error.** In `(a ] "oops`, the error is
  "unterminated string", not "unexpected closing delimiter". The Rust reader cuts the whole text
  into tokens before parsing anything, so a lexing error anywhere wins. The EELisp reader makes
  tokens on demand, so when it hits a syntax error it first lexes the rest of the text, and
  reports an unterminated string if it finds one.

Whether `0x+ff` *should* be 255 is a fair question. But it's a question about the language, to be
decided on its own. A second implementation must agree with the first, quirks included. Otherwise
the same file would mean two different things depending on which reader read it.

## Stage 2

With a reader in EELisp, the bootstrap gets a third stage:

```
stage 0   Rust reads,   the interpreted compiler compiles   → image 1
stage 1   Rust reads,   the compiled compiler compiles      → image 2
stage 2   EELisp reads, the compiled compiler compiles      → image 3
```

```rust
pub fn stage2(image: &str) -> Result<String, LispError> {
    let it = with_image(image)?;
    let forms = it.call("read-all", &[Value::Str(source())])…;   // the EELisp reader
    image_of(&it, forms)                                          // the EELisp compiler
}
```

The image is now made from both sources, the compiler and the reader, so the reader is compiled
by the compiler too. All three images are identical, byte for byte. Between the text of
`src/compiler.eel` and `src/reader.eel` and the bytecode in `boot/compiler.image`, every step is
EELisp. Rust provides the VM, the builtins and the characters of a string.

`cargo run --example bootstrap` now checks all three stages before it writes the image, and
`tests/bootstrap.rs` checks stage 2 on every test run.

## Two bugs the reader found

The image is EELisp text: one `(proto …)` form per line. That only works if everything printed
into it reads back as itself. Until this chapter, the compiler's source never contained anything
that broke that rule. The reader's source did, twice:

- **A dict with a key like `"("`.** The reader's table of punctuation is
  `{"(" :open "[" :open …}`. The printer wrote every key as a keyword, `{:( :open …}`, which is
  not readable: `:(` is a colon and then an open paren. The printer now writes a key that
  wouldn't read back as a keyword as a string instead, `{"(" :open}`, which the reader accepts.
  The compiler's `dict` instruction has the same rule for the keys it carries.
- **A string containing a newline.** `"\n"` was printed as a quote, a real line break, and a
  quote, which splits the image's one-proto-per-line layout. Readable printing now escapes `\n`,
  `\t` and `\r`, the escapes the reader understands. As a side effect, the REPL now shows
  `"a\nb"` instead of a string broken across two lines.

Both are print-and-read-back bugs, and both were there before. A self-hosted system runs its
parts against each other all the time. Here the printer writes the image and the reader reads it
back, so a gap between them shows up as a failed bootstrap.

## How fast

Reading the compiler's and the reader's source (33,000 characters, 89 forms), release build:

| | |
|---|---|
| the Rust reader | 0.78 ms |
| the EELisp reader, interpreted | 28 ms |
| the EELisp reader, compiled | 20.7 ms |
| for comparison: compiling those 89 forms | 7.6 ms |

The first version took 29 ms compiled. Counting calls, the method from chapter 8, showed about
430,000 calls to read 33,000 characters, so 13 per character. The native profile showed where
their time went: `malloc`, `free` and copying.

That's a property of the VM, not of the reader. In this engine a string value owns its text, so
copying even a one-character string allocates. Reading `(nth cs i)` copies a character, and so
does comparing it, and so does storing it. Keywords are the same, so every `:atom` compared
against a character's class was another allocation.

Three changes took it to 20.7 ms:

- Character classes are numbers (`reader-space` is 0, `reader-word` is 3), not keywords. The VM
  copies numbers for free since chapter 8.
- The prelude's `first` and `second` became the builtins `head` and `nth`. A prelude function is
  a whole call, and in an engine started from the image it's an *interpreted* one.
- Only atoms starting with a digit, a sign, a dot, `i` or `n` are tried as numbers. Most symbols
  now skip the number parsers.

What's left is about 400,000 calls, nearly all of them the irreducible work of looking at each
character: compare the position with the end, fetch the character, look up its class, add one.
The EELisp reader is 26 times slower than the Rust one, and reading takes longer than compiling.

That makes the case for the change chapter 8 named as the next big one: **the value
representation.** If strings and keywords shared their text (an `Rc<str>` instead of a `String`),
copying one would cost what copying a number costs. That would speed up every program that
handles text, the compiler included. It also changes a type that every host uses (eeditor matches
on `Value::Str`), so it's a step of its own.

## What it isn't yet

`read-all` is a function that compiled mode makes available. The engine still reads with Rust,
in both modes. Making the EELisp reader the engine's reader needs three more things:

- **Source positions.** `(source f)` shows a definition exactly as it was written, comments
  included, so the engine needs each top-level form's start and end in the text. The tokens
  already carry `start` and `end`, so that part is close.
- **The right kind of error.** The EELisp reader stops with `(error "unclosed list")`, a runtime
  error. The Rust one raises a parse error, and `(try … (catch e (dict-get e :kind)))` can tell
  them apart.
- **Speed**, per the section above. In compiled mode the engine would read every form before it
  compiled it, and reading currently costs more than twice as much as compiling.

Meanwhile the reader does its job: it proves that EELisp can read itself, it pins down what the
syntax actually is, and it makes stage 2 a test that runs on every build.
