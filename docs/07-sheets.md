# 7. Sheets

A sheet is a spreadsheet whose formulas are EELisp. There's no separate formula language. A
formula is one expression after `=`, and inside it `B1` is a cell's value and `B1:B9` is a list
of values. Everything in the rest of these pages works inside a formula: `map`, `filter`,
`date-add`, even `count-records` against the database.

Each sheet is a SQLite file with the extension `.eesheet`. A sheet is named `"Budget"`,
`"money/Budget"` or by absolute path. A relative name resolves against the host's workspace
folder, `(current-dir)`, and `.eesheet` is implied.

## A first sheet

```lisp
(sheet-new "Budget")
(sheet-set "Budget" "A1"
           '(("rent"  "1200")
             ("food"  "450")
             ("total" "=(sum B1:B2)")))     ; a list of rows types a block

(sheet-get "Budget" "B3")                   ; → 1650
(sheet-set "Budget" "B2" "500")             ; B3 recalculates by itself
(sheet-get "Budget" "B3")                   ; → 1700
(sheet-rows "Budget" "A1:B3")               ; → (("rent" 1200) ("food" 500) ("total" 1700))
```

`sheet-set` returns the cells that changed, each one as `(row col input value error fmt)` with
rows and columns counted from 0. A host uses that list to repaint only what moved.

## What a cell holds

A cell keeps what was typed, its *input*, and the last value that input produced. How the input
is read:

| You type | It becomes |
|---|---|
| `1200` | the number 1200 |
| `rent` | the text "rent" |
| `'42` | the text "42", since a leading `'` forces text |
| `=(sum B1:B2)` | a formula |
| `50%` | 0.5, shown as a percentage |
| `$1,200.50`, `R$ 1.200,50` | money: 1200.5, with a currency format |
| `1 200,50` | 1200.5 |
| `2026-09-16` | a date, which sorts and takes part in date arithmetic |
| `16/09/2026` | text, because the sheet won't guess which part is the month |

Whichever of `.` or `,` comes last is taken as the decimal point. A lone comma before exactly
three digits separates thousands, so `1,200` is twelve hundred and `1,5` is one and a half. A
format already set on the cell wins over the guess.

## Inside a formula

| You write | It means |
|---|---|
| `B1` | the value in B1 (capitals only, since `b1` is an ordinary name) |
| `B1:B9` | the range as a flat list, with blanks as `nil` (one row or one column) |
| `A1:C9` | a block wider and taller than one: a list of rows, `(("apple" 3) ("pear" 5) …)` |
| `E:E`, `B:D` | whole columns, read down to the last filled row — `E:E` flat, `B:D` as rows |
| `$B$1`, `B$1`, `$B1` | pinned whole, by row, or by column |
| `Rates!A1`, `Rates!A1:A9` | a cell or range in the sheet named `Rates`, in the same folder |
| `#REF!` | what a reference turns into when its cell is deleted |

```lisp
=(sum B1:B9)
=(round (/ B1 $B$3) 2)                         ; a share of the total; the pin survives filling
=(sum (filter (fn (x) (> x 400)) B1:B3))
=(length (filter (fn (x) (= x "Work")) E:E))   ; how many rows say Work
=(* B2 Rates!A1)                               ; another sheet
=(date-format (today) "yyyy-MM-dd")
=(count-records people :where "city = ?" :params '("Lisbon"))
```

`sum`, `avg`, `min` and `max` skip blanks and text, so a range with labels in it still adds up —
and they reach into a block's rows, so `(sum A1:C9)` adds every number in it. A formula inside the
column it reads (`=(sum E:E)` in E5) is a circular reference, as it would be anywhere else.

## Recalculation

When a cell changes, the engine works out everything downstream of it and recomputes each
formula after the formulas it reads. Results are stored in the file, so **opening a sheet runs no
formula**. A sheet with ten thousand formulas opens as fast as one with none.

A formula that reads the **database** follows it too. While a formula runs, the engine notes every
table it reads — through `query`, `count-records`, the agenda's `items`, or a function of yours that
calls them — and keeps that list with the cell, in the file. When something writes one of those
tables (`insert`, `update`, `delete`), the formulas of open sheets that read it are recalculated
once the write's top-level form has finished — a loop of a hundred inserts recalculates once, at
the end:

```lisp
(sheet-set "Portfolio" "D2" "=(latest A2)")    ; latest queries the prices table
(insert prices {:symbol "bitcoin" :price 80121 :at "2026-10-26 09:30"})
(sheet-get "Portfolio" "D2")                   ; → 80121, nothing pressed
```

What a formula reads is noted when it runs, and kept in the file, so a sheet just opened — which
runs nothing — still knows. A sheet last saved by an older engine has no such list yet: its
formulas learn what they read at its next recalculation (**↻** once does it). Formulas that read the clock, or a sheet that was closed when it
changed, still need **↻** (`sheet-recalc`).

Formulas that read each other in a circle are found with Tarjan's strongly-connected-components
algorithm, run without recursion so that a long chain of running totals can't overflow the stack.
Every cell in the circle gets the same error rather than a wrong number:

```lisp
(sheet-set "Budget" "F1" "=(+ 1 G1)")
(sheet-set "Budget" "G1" "=(+ 1 F1)")
; both cells: error "circular reference between F1, G1"
```

```mermaid
sequenceDiagram
    participant H as Host or your code
    participant S as sheet-set
    participant P as Sheet (plan)
    participant E as Evaluator
    participant O as Other open sheets
    H->>S: (sheet-set "Budget" "B2" "500")
    S->>P: stage the new input for B2
    S->>P: plan(B2): B2 and everything that reads it, transitively
    P-->>S: steps in order: Compute B3, Compute C1 … or Cycle [F1 G1]
    loop each step
        alt Compute cell
            S->>S: read the Sheet!A1 cells it names in other sheets
            S->>E: eval the formula, its cell names bound to values
            E-->>S: value or error
        else Cycle
            S->>S: "circular reference" on every cell of the cycle
        end
    end
    S->>S: save the changed cells to Budget.eesheet in one write
    S->>O: recompute open sheets that read Budget, once each
    S-->>H: changed cells as (row col input value error fmt)
```

Writing to a sheet also recomputes the other **open** sheets that read it, once each, so two
sheets naming each other settle down instead of bouncing back and forth. A formula that reads the
database, the clock or a sheet that wasn't open isn't tracked. Run `(sheet-recalc "Budget")` to
recompute everything.

## Moving things around

```lisp
(sheet-fill "Budget" "C1" "C2:C12")            ; copy C1 down; references shift per row
(sheet-copy "Budget" "B1:B3")                  ; → what was typed, as rows of text
(sheet-paste "Budget" "D1" (sheet-copy "Budget" "B1:B3") "B1")
                                               ; paste; formulas move by the distance travelled
(sheet-insert-rows "Budget" 3)                 ; rows numbered from 1; formulas follow their cells
(sheet-delete-rows "Budget" 3 2)               ; a reference to a deleted cell reads #REF!
(sheet-insert-cols "Budget" "B")
(sheet-delete-cols "Budget" "B" 2)
```

When rows or columns move, the engine rewrites the references in each formula's **text** and
keeps its spacing and comments. `=(sum B1:B2)` becomes `=(sum B2:B3)`, and nothing else in the
formula changes.

## Data in, data out

```lisp
(sheet-put "Report" "A1" (query people))       ; a header row, then the records
(sheet-put "Report" "A1" '((1 2) (3 4)))       ; values stay values: never re-read as input
(sheet-open "Budget")    ; everything a host needs to draw it: {:path :version :cells :widths :heights}
(sheet-version "Budget") ; a counter that moves on every write
(sheet-bytes "Budget")   ; the whole file as base64
(sheet-from-bytes "Copy" (sheet-bytes "Budget"))
(sheet-close "Budget")   ; let go of the file before renaming or deleting it
```

## Formatting

Formats are stored with the cell and are for the host to apply. The engine only keeps them and
moves them around.

```lisp
(sheet-format "Budget" "B1:B9" {:num "currency" :dp 2})
(sheet-format "Budget" "A1" {:bold true :bg "#fff3c4"})
(sheet-format "Budget" "A1:A9" nil)            ; clear
(sheet-col-width "Budget" "A" 160)
(sheet-row-height "Budget" 3 48)
```

The keys are `:num`, `:dp`, `:cur`, `:date`, `:bold`, `:italic`, `:align`, `:wrap`, `:bg`, `:fg`
and `:border`.

Continue with [A self-documenting language](08-self-documenting.md).
