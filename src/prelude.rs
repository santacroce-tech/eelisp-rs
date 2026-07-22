//! Standard library written in EELisp itself, loaded at startup (ANALYSIS §4.7).
//! `when`/`unless` use macro rest-params and quasiquote — both work here (bugs fixed).

pub const PRELUDE: &str = r#"
(defn id (x) x)
(defn inc (n) (+ n 1))
(defn dec (n) (- n 1))

(defn even? (n) (= 0 (mod n 2)))
(defn odd?  (n) (not (even? n)))
(defn zero? (n) (= n 0))
(defn pos?  (n) (> n 0))
(defn neg?  (n) (< n 0))

(defn first  (lst) (head lst))
(defn second (lst) (nth lst 1))
(defn third  (lst) (nth lst 2))
(defn last   (lst)
  (if (empty? (tail lst))
    (head lst)
    (last (tail lst))))

(defn take (n lst)
  (if (or (= n 0) (empty? lst))
    (list)
    (cons (head lst) (take (- n 1) (tail lst)))))

(defn drop (n lst)
  (if (or (= n 0) (empty? lst))
    lst
    (drop (- n 1) (tail lst))))

(defn count (lst) (length lst))

(defn some? (pred lst)
  (if (empty? lst)
    false
    (if (pred (head lst)) true (some? pred (tail lst)))))

(defn every? (pred lst)
  (if (empty? lst)
    true
    (if (pred (head lst)) (every? pred (tail lst)) false)))

(defn compose (f g) (fn (x) (f (g x))))

(defmacro when (test . body)
  `(if ,test (do ,@body) nil))

(defmacro unless (test . body)
  `(if ,test nil (do ,@body)))
"#;
