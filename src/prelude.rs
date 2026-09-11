//! Standard library written in EELisp itself, loaded at startup (ANALYSIS §4.7).
//! `when`/`unless` use macro rest-params and quasiquote — both work here (bugs fixed).
//!
//! The `;;` comments are load-bearing: the parser hands each definition's comment block to the
//! source index, so `(source take)` reads back the note written here.

pub const PRELUDE: &str = r#"
;; The identity function — returns its argument unchanged.
(defn id (x) x)

;; One more.  (inc 4) → 5
(defn inc (n) (+ n 1))

;; One less.  (dec 4) → 3
(defn dec (n) (- n 1))

;; Is n divisible by two?
(defn even? (n) (= 0 (mod n 2)))

;; Is n not divisible by two?
(defn odd?  (n) (not (even? n)))

;; Is n exactly zero?
(defn zero? (n) (= n 0))

;; Is n greater than zero?
(defn pos?  (n) (> n 0))

;; Is n less than zero?
(defn neg?  (n) (< n 0))

;; The first element of a list.
(defn first  (lst) (head lst))

;; The second element of a list.
(defn second (lst) (nth lst 1))

;; The third element of a list.
(defn third  (lst) (nth lst 2))

;; The last element of a list.
(defn last   (lst)
  (if (empty? (tail lst))
    (head lst)
    (last (tail lst))))

;; The first n elements.  (take 2 '(1 2 3)) → (1 2)
(defn take (n lst)
  (if (or (= n 0) (empty? lst))
    (list)
    (cons (head lst) (take (- n 1) (tail lst)))))

;; Everything after the first n elements.  (drop 2 '(1 2 3)) → (3)
(defn drop (n lst)
  (if (or (= n 0) (empty? lst))
    lst
    (drop (- n 1) (tail lst))))

;; How many elements a list has — the same as length.
(defn count (lst) (length lst))

;; Does any element satisfy pred?
(defn some? (pred lst)
  (if (empty? lst)
    false
    (if (pred (head lst)) true (some? pred (tail lst)))))

;; Do all elements satisfy pred?
(defn every? (pred lst)
  (if (empty? lst)
    true
    (if (pred (head lst)) (every? pred (tail lst)) false)))

;; Combines two functions: (compose f g) is the function x → (f (g x)).
(defn compose (f g) (fn (x) (f (g x))))

;; Run the body only when test is true; otherwise nil.
(defmacro when (test . body)
  `(if ,test (do ,@body) nil))

;; Run the body only when test is false; otherwise nil.
(defmacro unless (test . body)
  `(if ,test nil (do ,@body)))
"#;
