# eelisp.app

The website, as plain HTML. No build step, no dependencies — edit a file, deploy the folder.

```
index.html      what EELisp is, and why
guide.html      the language, in order, with runnable examples
reference.html  every special form and function
embed.html      using the engine from Rust or over JSON
docs.html       the full documentation on one page — generated, don't edit it here
assets/style.css   every page's styles — edit here, not in the pages
```

`docs.html` is built from the chapters in `docs/` by `python3 scripts/build-docs.py`, which also
regenerates `docs/reference.md` from the engine's manual. Edit the Markdown, rebuild, and commit
both. It carries its own styles and inline diagrams, so it doesn't use `assets/style.css`.

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
grep -oE '"sheet-[a-z-]+"' src/sheet_builtins.rs | sort -u              # sheets
grep -oE '\(def(n|macro)? +[a-z][a-z0-9!?*<>=/-]*' src/prelude.rs    # prelude
```

## Deploying

There is no deploy step: **merging to `main` publishes the site.** `.github/workflows/pages.yml`
runs on every push to `main` that touches `site/`, copies the folder (leaving this README out) and
hands it to GitHub Pages, which serves it at eelisp.app. Watch it in the repo's Actions tab, or run
it by hand there (*Pages → Run workflow*). Only `main` goes live, so a branch open for review never
reaches the site by accident.

The domain is set in the repo's **Settings → Pages** (custom domain `eelisp.app`, *Enforce HTTPS*
on). The DNS at the registrar points at GitHub: `A` records for `185.199.108.153`, `.109.153`,
`.110.153` and `.111.153`, `AAAA` records for `2606:50c0:8000::153` through `8003::153`, and `www` as a
`CNAME` to `santacroce-tech.github.io`. A workflow-published site ignores a `CNAME` file, so there
isn't one.
