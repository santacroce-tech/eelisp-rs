# eelisp.app

The website, as plain HTML. No build step, no dependencies — edit a file, deploy the folder.

```
index.html      what EELisp is, and why
guide.html      the language, in order, with runnable examples
reference.html  every special form and function
embed.html      using the engine from Rust or over JSON
assets/style.css   every page's styles — edit here, not in the pages
```

## Editing

Shares its stylesheet design with [eeditor.app](https://eeditor.app); only `--accent` and
`--accent-soft` differ, so the two sites read as siblings. Colours are CSS variables at the top of
`assets/style.css`, defined once for light and once for dark.

Code samples use three classes for colour: `.cm` comment, `.st` string, `.kw` keyword.

**Every code sample on this site was run against the engine before publishing.** Keep it that way —
`cargo build --release --bin eelisp && ./target/release/eelisp yourfile.eelisp`. Several forms are
easy to get wrong from memory:

| Looks right | Actually |
|---|---|
| `(insert t :a 1)` | `(insert t {:a 1})` — a dict |
| `(deftable t (a :type string) (b :type number))` | `(deftable t (a:string b:number))`, or the long form inside **one** list |
| `(defrule r (match "x") (assign "c"))` | `(defrule r :when (str-matches text "x") :assign "c")` |
| `match` as a regex test | `match` returns capture groups; `str-matches` tests |
| `(for-each f lst)` | `(for-each x lst body…)` — a special form |
| `~x` to unquote | `,x` and `,@x` |

## Keeping the reference honest

The function tables are generated from the source by hand today. To re-check them after adding
builtins:

```bash
grep -o 'b!("[^"]*"' src/builtins.rs | sed 's/b!("//;s/"//'          # core
grep -oE 'define_db\(env, "[^"]+"' src/db_builtins.rs                # database
grep -oE 'defb\(env, "[^"]+"' src/agenda_builtins.rs                 # agenda
grep -oE '\(def(n|macro)? +[a-z][a-z0-9!?*<>=/-]*' src/prelude.rs    # prelude
```

## Deploying

```bash
rsync -avz --delete -e "ssh -i ~/.ssh/id_epitetus" \
  site/ root@91.98.47.97:/var/www/eelisp.app/
```
