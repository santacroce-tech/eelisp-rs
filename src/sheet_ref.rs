//! Cell addressing for sheets: `A1` ⇄ (row, col), ranges, and what a symbol in a formula refers
//! to. Also the one piece of text surgery sheets need — moving the references in a formula when
//! rows or columns are inserted or deleted — done on the formula's **text**, so the spacing and
//! comments the user wrote survive.
//!
//! Rows and columns are 0-based here; `A1` is (0, 0). A reference inside a formula is uppercase
//! (`B3`, `$B$3`, `B3:C9`); lowercase `b3` stays an ordinary symbol.

use crate::lexer::{lex_spanned, Token};

pub const MAX_ROWS: u32 = 1_048_576;
/// `A` … `ZZZ`.
pub const MAX_COLS: u32 = 18_278;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CellRef {
    // field order is the sort order: row-major, which is how a range reads
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }
    pub fn a1(&self) -> String {
        format!("{}{}", col_name(self.col), self.row + 1)
    }
}

/// A rectangle of cells, `start` top-left and `end` bottom-right (both included).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: CellRef,
    pub end: CellRef,
}

impl Range {
    /// The rectangle spanned by two corners, in either order.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Range {
            start: CellRef::new(a.row.min(b.row), a.col.min(b.col)),
            end: CellRef::new(a.row.max(b.row), a.col.max(b.col)),
        }
    }
    pub fn contains(&self, c: CellRef) -> bool {
        (self.start.row..=self.end.row).contains(&c.row) && (self.start.col..=self.end.col).contains(&c.col)
    }
    pub fn height(&self) -> u32 {
        self.end.row - self.start.row + 1
    }
    pub fn width(&self) -> u32 {
        self.end.col - self.start.col + 1
    }
    pub fn len(&self) -> u64 {
        self.height() as u64 * self.width() as u64
    }
    pub fn is_empty(&self) -> bool {
        false // a range always holds at least its one corner
    }
    /// Every cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = CellRef> {
        let (r0, r1, c0, c1) = (self.start.row, self.end.row, self.start.col, self.end.col);
        (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| CellRef::new(r, c)))
    }
    pub fn a1(&self) -> String {
        if self.start == self.end {
            self.start.a1()
        } else {
            format!("{}:{}", self.start.a1(), self.end.a1())
        }
    }
}

/// 0 → `A`, 25 → `Z`, 26 → `AA`.
pub fn col_name(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

/// `A` → 0, `AA` → 26. Uppercase letters only, at most three.
pub fn col_index(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let n = letters.bytes().fold(0u32, |acc, b| acc * 26 + (b - b'A' + 1) as u32);
    (n <= MAX_COLS).then(|| n - 1)
}

/// A cell reference as written — the `$` anchors are kept so a rewrite prints it back the same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Anchored {
    pub cell: CellRef,
    pub abs_col: bool,
    pub abs_row: bool,
}

impl Anchored {
    fn print(&self) -> String {
        format!(
            "{}{}{}{}",
            if self.abs_col { "$" } else { "" },
            col_name(self.cell.col),
            if self.abs_row { "$" } else { "" },
            self.cell.row + 1
        )
    }
}

/// `B3`, `$B3`, `B$3`, `$B$3`.
pub fn parse_cell(s: &str) -> Option<Anchored> {
    let (abs_col, s) = match s.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let letters_end = s.bytes().position(|b| !b.is_ascii_uppercase())?;
    let col = col_index(&s[..letters_end])?;
    let (abs_row, digits) = match s[letters_end..].strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, &s[letters_end..]),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (row <= MAX_ROWS).then(|| Anchored { cell: CellRef::new(row - 1, col), abs_col, abs_row })
}

/// `B3:C9` (anchors allowed). A single cell is not a range here — see `parse_area`.
pub fn parse_range(s: &str) -> Option<Range> {
    let (a, b) = s.split_once(':')?;
    Some(Range::new(parse_cell(a)?.cell, parse_cell(b)?.cell))
}

/// An area named by a host or a caller: `C3` or `A1:C9`, case-insensitive. Formulas are stricter.
pub fn parse_area(s: &str) -> Option<Range> {
    let up = s.trim().to_ascii_uppercase();
    match parse_range(&up) {
        Some(r) => Some(r),
        None => parse_cell(&up).map(|a| Range::new(a.cell, a.cell)),
    }
}

/// What a symbol inside a formula refers to, if it refers to anything.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RefSym {
    Cell(CellRef),
    Range(Range),
    /// `Budget!A1` — cells in another sheet, named beside this one. `at` is a Cell or a Range.
    Other { sheet: String, at: Box<RefSym> },
    /// What a reference to a deleted cell becomes.
    Deleted,
}

pub const DELETED: &str = "#REF!";

pub fn classify(sym: &str) -> Option<RefSym> {
    if sym == DELETED {
        return Some(RefSym::Deleted);
    }
    if let Some((sheet, rest)) = sym.split_once('!') {
        if sheet.is_empty() {
            return None;
        }
        let at = parse_cell(rest).map(|a| RefSym::Cell(a.cell)).or_else(|| parse_range(rest).map(RefSym::Range))?;
        return Some(RefSym::Other { sheet: sheet.to_string(), at: Box::new(at) });
    }
    if let Some(a) = parse_cell(sym) {
        return Some(RefSym::Cell(a.cell));
    }
    parse_range(sym).map(RefSym::Range)
}

// ── inserting and deleting rows / columns ────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Rows,
    Cols,
}

/// `n` rows or columns inserted before index `at`, or deleted starting at it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Insert { axis: Axis, at: u32, n: u32 },
    Delete { axis: Axis, at: u32, n: u32 },
}

impl Edit {
    pub fn axis(&self) -> Axis {
        match self {
            Edit::Insert { axis, .. } | Edit::Delete { axis, .. } => *axis,
        }
    }

    fn limit(&self) -> u32 {
        match self.axis() {
            Axis::Rows => MAX_ROWS,
            Axis::Cols => MAX_COLS,
        }
    }

    /// Where index `i` on the edited axis ends up; `None` when it was deleted (or pushed off the end).
    pub fn map_index(&self, i: u32) -> Option<u32> {
        match *self {
            Edit::Insert { at, n, .. } => {
                if i < at {
                    Some(i)
                } else {
                    let moved = i as u64 + n as u64;
                    (moved < self.limit() as u64).then_some(moved as u32)
                }
            }
            Edit::Delete { at, n, .. } => {
                let end = at as u64 + n as u64;
                if i < at {
                    Some(i)
                } else if (i as u64) < end {
                    None
                } else {
                    Some(i - n)
                }
            }
        }
    }

    pub fn map_cell(&self, c: CellRef) -> Option<CellRef> {
        match self.axis() {
            Axis::Rows => self.map_index(c.row).map(|row| CellRef::new(row, c.col)),
            Axis::Cols => self.map_index(c.col).map(|col| CellRef::new(c.row, col)),
        }
    }

    /// The span `[lo, hi]` on the edited axis after the edit. An insertion inside the span grows
    /// it; a deletion trims it. `None` when every index in it was deleted.
    fn map_span(&self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        match *self {
            Edit::Insert { .. } => Some((self.map_index(lo)?, self.map_index(hi).unwrap_or(self.limit() - 1))),
            Edit::Delete { at, n, .. } => {
                let end = at as i64 + n as i64; // first index after the deleted block
                let new_lo = if (lo as i64) < at as i64 {
                    lo as i64
                } else if (lo as i64) >= end {
                    lo as i64 - n as i64
                } else {
                    at as i64 // the first survivor after the block moves up to `at`
                };
                let new_hi = if (hi as i64) < at as i64 {
                    hi as i64
                } else if (hi as i64) >= end {
                    hi as i64 - n as i64
                } else {
                    at as i64 - 1 // the last survivor before the block
                };
                (new_lo <= new_hi).then_some((new_lo as u32, new_hi as u32))
            }
        }
    }
}

/// A formula's text after an edit.
#[derive(Debug, PartialEq)]
pub struct Rewrite {
    pub text: String,
    /// The text differs from what it was.
    pub changed: bool,
    /// A reference now covers different cells than before — a range grew or shrank, or a cell
    /// was deleted — so the formula has to be recalculated. A reference that merely moved along
    /// with its cell does not count: it still reads the same value.
    pub resized: bool,
}

/// Move the references in a formula body (the text after `=`) to where their cells went.
/// Strings, comments and everything that isn't a cell reference are left exactly as they were.
pub fn rewrite_formula(body: &str, edit: &Edit) -> Rewrite {
    let mut resized = false;
    let mut rw = rewrite_symbols(body, |sym| rewrite_symbol(sym, edit, &mut resized));
    rw.resized = resized;
    rw
}

/// Move every reference by `(dr, dc)` — what copying a formula to another cell does. A `$` holds a
/// coordinate still; a reference pushed off the sheet becomes `#REF!`.
pub fn shift_formula(body: &str, dr: i64, dc: i64) -> Rewrite {
    let mut lost = false;
    let shift = |a: Anchored| -> Option<Anchored> {
        let row = if a.abs_row { a.cell.row as i64 } else { a.cell.row as i64 + dr };
        let col = if a.abs_col { a.cell.col as i64 } else { a.cell.col as i64 + dc };
        let ok = (0..MAX_ROWS as i64).contains(&row) && (0..MAX_COLS as i64).contains(&col);
        ok.then(|| Anchored { cell: CellRef::new(row as u32, col as u32), ..a })
    };
    let mut rw = rewrite_symbols(body, |sym| {
        if sym.contains('!') {
            return None; // another sheet's cells, or an already-deleted reference
        }
        if let Some(a) = parse_cell(sym) {
            return Some(match shift(a) {
                Some(moved) => moved.print(),
                None => {
                    lost = true;
                    DELETED.to_string()
                }
            });
        }
        let (left, right) = sym.split_once(':')?;
        match (shift(parse_cell(left)?), shift(parse_cell(right)?)) {
            (Some(a), Some(b)) => Some(format!("{}:{}", a.print(), b.print())),
            _ => {
                lost = true;
                Some(DELETED.to_string())
            }
        }
    });
    rw.resized = lost;
    rw
}

/// Replace the reference-shaped symbols in a formula's text, leaving strings, comments, spacing and
/// everything else exactly as written. `replace` returns the new spelling of a symbol, or None to
/// leave it alone.
fn rewrite_symbols(body: &str, mut replace: impl FnMut(&str) -> Option<String>) -> Rewrite {
    let Ok(tokens) = lex_spanned(body) else {
        // a formula that doesn't read is already an error; leave it be
        return Rewrite { text: body.to_string(), changed: false, resized: false };
    };
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for t in tokens {
        let Token::Sym(sym) = &t.tok else { continue };
        let Some(replacement) = replace(sym) else { continue };
        out.extend(&chars[last..t.start]);
        out.push_str(&replacement);
        last = t.end;
    }
    out.extend(&chars[last..]);
    let changed = out != body;
    Rewrite { text: out, changed, resized: false }
}

/// The new spelling of one symbol, or `None` when it isn't a reference this edit moves.
fn rewrite_symbol(sym: &str, edit: &Edit, resized: &mut bool) -> Option<String> {
    if sym.contains('!') {
        return None; // another sheet's cells, or an already-deleted reference
    }
    if let Some(a) = parse_cell(sym) {
        return Some(match edit.map_cell(a.cell) {
            Some(cell) => Anchored { cell, ..a }.print(),
            None => {
                *resized = true;
                DELETED.to_string()
            }
        });
    }
    let (left, right) = sym.split_once(':')?;
    let (a, b) = (parse_cell(left)?, parse_cell(right)?);
    // Normalise the corners, carrying each coordinate's anchor with it.
    let (top, bottom) = if a.cell.row <= b.cell.row { (a, b) } else { (b, a) };
    let (lft, rgt) = if a.cell.col <= b.cell.col { (a, b) } else { (b, a) };
    let (r0, r1, c0, c1) = (top.cell.row, bottom.cell.row, lft.cell.col, rgt.cell.col);
    let before = Range::new(CellRef::new(r0, c0), CellRef::new(r1, c1));
    let after = match edit.axis() {
        Axis::Rows => edit.map_span(r0, r1).map(|(lo, hi)| (lo, hi, c0, c1)),
        Axis::Cols => edit.map_span(c0, c1).map(|(lo, hi)| (r0, r1, lo, hi)),
    };
    let Some((r0, r1, c0, c1)) = after else {
        *resized = true;
        return Some(DELETED.to_string());
    };
    if Range::new(CellRef::new(r0, c0), CellRef::new(r1, c1)).len() != before.len() {
        *resized = true;
    }
    let start = Anchored { cell: CellRef::new(r0, c0), abs_col: lft.abs_col, abs_row: top.abs_row };
    let end = Anchored { cell: CellRef::new(r1, c1), abs_col: rgt.abs_col, abs_row: bottom.abs_row };
    Some(format!("{}:{}", start.print(), end.print()))
}
