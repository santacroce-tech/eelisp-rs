# 4. Working with data

Every data operation in EELisp returns a new value and leaves its input alone. Adding to a list
gives a new list, and setting a dict key gives a new dict. That makes values safe to share, and
it makes the host boundary simple, because a value handed to a host is never changed afterwards.

## Numbers

There's one numeric type, a 64-bit float. Whole results print without a decimal point.

```lisp
(+ 1 2 3)        ; → 6          (- 10 3 2)   ; → 5
(* 2 3 4)        ; → 24         (/ 7 2)      ; → 3.5
(mod 7 3)        ; → 1          (abs -4)     ; → 4
(pow 2 10)       ; → 1024       (expt 2 10)  ; → 1024
(floor 2.7)      ; → 2          (ceil 2.1)   ; → 3
(round 2.5)      ; → 3          halves round away from zero

(< 1 2 3)        ; → true       comparisons take any number of arguments
(= 1 1.0)        ; → true
```

`min`, `max`, `sum` and `avg` look inside lists and skip `nil` and text. That's what a
spreadsheet range needs, since a range can contain blanks and labels:

```lisp
(sum 1 (list 2 nil "x" 3))      ; → 6
(min 3 (list 1 nil 2))          ; → 1
(avg (list 2 4 nil))            ; → 3
```

## Lists

```lisp
(list 1 2 3)                    ; → (1 2 3)
[1 2 3]                         ; the same
(cons 0 (list 1 2))             ; → (0 1 2)
(append (list 1) (list 2 3))    ; → (1 2 3)
(reverse (list 1 2 3))          ; → (3 2 1)

(head lst)   (car lst)          ; the first element — nil for an empty list
(tail lst)   (cdr lst)          ; everything after it
(nth lst 1)                     ; 0-based — nil when out of range
(first l) (second l) (third l) (last l)
(take 2 l)   (drop 2 l)
(length l)   (count l)   (empty? l)

(range 3)                       ; → (0 1 2)
(range 1 5)                     ; → (1 2 3 4)       end-exclusive
(range 5 0 -1)                  ; → (5 4 3 2 1)
```

Higher-order functions do most of the work:

```lisp
(map (fn (x) (* 2 x)) (range 1 5))          ; → (2 4 6 8)
(filter even? (range 1 10))                 ; → (2 4 6 8)
(reduce (fn (a b) (+ a b)) 0 (range 1 5))   ; → 10
(sort-by (fn (p) (nth p 1)) (list (list "a" 3) (list "b" 1)))
                                            ; → (("b" 1) ("a" 3))
(zip (list 1 2) (list "a" "b"))             ; → ((1 "a") (2 "b"))
(some? even? (list 1 3 4))                  ; → true
(every? even? (list 2 4))                   ; → true
(apply + (list 1 2 3))                      ; → 6
```

## Dicts

A dict maps keys to values and remembers the order the keys were added in. Keys are usually
keywords, but a string works as well: `(dict-get d "name")` finds `:name`.

```lisp
(def d {:name "Ada" :born 1815})
(dict :name "Ada" :born 1815)       ; the same, built by a call

(dict-get d :name)                  ; → "Ada"
(dict-get d :missing)               ; → nil
(dict-has d :name)                  ; → true
(dict-keys d)                       ; → (:name :born)
(dict-values d)                     ; → ("Ada" 1815)

(dict-set d :born 1816)             ; → {:name "Ada" :born 1816} — d itself is unchanged
(dict-merge {:a 1 :b 2} {:b 3 :c 4})    ; → {:a 1 :b 3 :c 4} — later keys win
```

## Strings

```lisp
(str "x=" 42 " " :k)              ; → "x=42 :k" — str converts anything
(str-len "hello")                 ; → 5 (characters, not bytes)
(str-upper "hi") (str-lower "Hi") (str-trim "  hi  ")
(substr "hello" 1 3)              ; → "el" — 0-based, end-exclusive
(str-split "a,b,c" ",")           ; → ("a" "b" "c")
(str-join "-" (list "a" "b"))     ; → "a-b" — either argument order works
(str-replace "14:30" ":" "")      ; → "1430" — every occurrence
(str-contains "hello" "ell")      ; → true
(str-starts-with "hello" "he")    ; → true
(str-ends-with "hello" "lo")      ; → true
(str-matches "a1" "^[a-z][0-9]$") ; → true — a regular expression
```

## Types and conversion

```lisp
(type 42)                         ; → "number"
(number? x) (string? x) (bool? x) (list? x)
(nil? x) (symbol? x) (keyword? x) (fn? x)     ; fn? is true for builtins too

(->string 42)                     ; → "42"
(->number "42")                   ; → 42 — an error for text that isn't a number
(->bool 0)                        ; → true — only false and nil are false
```

## Dates

The engine does its own date arithmetic and has no external date library. Two spellings of a date
are in use:

- **Epoch seconds**, a number. This is what `(now)` and `(today)` return.
- **`"YYYY-MM-DD"` strings.** Agenda items, `date-add` and `date-diff` use these.

`date-format` accepts either spelling.

```lisp
(now)                                   ; → 1790351486.65   the current instant
(today)                                 ; → 1790294400      midnight today
(date-format (today))                   ; → "2026-09-25 00:00"
(date-format "2026-09-25" "EEEE")       ; → "Friday"
(date-format "2026-09-25" "d MMMM yyyy"); → "25 September 2026"

(date-add "2026-09-25" 7 :days)         ; → "2026-10-02"
(date-add "2026-09-25" 2 :weeks)        ; → "2026-10-09"
(date-add "2026-01-31" 1 :months)       ; → "2026-02-28" — clamped to the month's end
(date-diff "2026-10-01" "2026-09-25")   ; → 6 — the first date minus the second, in days
```

Format patterns follow the Unicode convention:

| Pattern | Gives | Pattern | Gives |
|---|---|---|---|
| `yyyy` / `yy` | 2026 / 26 | `HH` / `H` | hour, 00–23 |
| `MM` / `M` | 09 / 9 | `hh` / `h` | hour, 01–12 |
| `MMM` / `MMMM` | Sep / September | `mm` | minutes |
| `dd` / `d` | 05 / 5 | `ss` | seconds |
| `EEE` / `EEEE` | Fri / Friday | `'text'` | literal text |

## JSON

```lisp
(json-stringify (list 1 "a" nil true {:k :v}))   ; → "[1,\"a\",null,true,{\"k\":\"v\"}]"
(json-parse "[1, null, true, {\"x\": {\"y\": 2}}]")
                                                 ; → (1 nil true {:x {:y 2}})
(dict-get (json-parse "{\"name\":\"Ada\"}") :name)   ; → "Ada"
```

JSON objects become dicts, with their keys sorted. Arrays become lists, and `null` becomes `nil`.

## The network

```lisp
(http-get "https://example.com/api")            ; → the response body, a string
(http-post "https://example.com/api" body)
```

These are the only functions that reach the network, and they're synchronous. A failure is an
error that names the URL. The WebAssembly build leaves them out, because a browser has its own
`fetch`. There, the functions say they're unavailable instead of crashing.

## Output

```lisp
(print "a" "b")        ; writes "a b" with no newline
(println "total:" 42)  ; writes "total: 42" and a newline
```

Arguments are separated by spaces, and strings are written without their quotes. Where the text
goes depends on the host. The command line echoes it to the terminal. An embedding host *captures*
it, and it comes back in the `output` field of the JSON envelope (see
[Embedding](10-embedding.md)).

Continue with [The database](05-database.md).
