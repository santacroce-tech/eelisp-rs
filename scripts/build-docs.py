#!/usr/bin/env python3
"""Build the EELisp documentation.

    python3 scripts/build-docs.py

1. Regenerates docs/reference.md from the engine's own manual (src/docs.rs) and prelude
   (src/prelude.rs), the same tables `(functions)` and `(source …)` read.
2. Builds docs/eelisp.html: every chapter in one self-contained page. Mermaid sequence diagrams
   are drawn as inline SVG and images are inlined, so the page makes no network requests.
3. Builds site/docs.html: the same page for eelisp.app, with the site's navigation and favicon.

Standard library only (Python 3.9+), no Markdown package needed: the converter understands the
subset of Markdown the chapters use.
"""

import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"

CHAPTERS = [
    "README.md",
    "01-introduction.md",
    "02-syntax.md",
    "03-language.md",
    "04-data.md",
    "05-database.md",
    "06-agenda.md",
    "07-sheets.md",
    "08-self-documenting.md",
    "09-architecture.md",
    "10-embedding.md",
    "reference.md",
]


# ── the reference, from src/docs.rs and src/prelude.rs ──────────────────────────────────────────

TABLE_TITLES = {
    "SPECIAL_FORMS": "Special forms",
    "CORE": None,  # split by its `// ── section ──` comments
    "DATABASE": "Database",
    "AGENDA": "Agenda",
    "SHEETS": "Sheets",
    "EDITOR": "Editor RPC",
}

SECTION_TITLES = {
    "arithmetic": "Arithmetic",
    "comparison & logic": "Comparison and logic",
    "strings": "Strings",
    "lists": "Lists",
    "dicts": "Dicts",
    "types & conversion": "Types and conversion",
    "dates": "Dates",
    "evaluation, JSON, network": "Evaluation, JSON and the network",
    "functions / source": "Finding your way around",
}

RUST_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "'": "'", "0": "\0"}


def rust_strings(src, i):
    """Read the string-literal arguments of a doc!(…) call starting after its '('.
    Returns (strings, index after the closing paren)."""
    out = []
    depth = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c == '"':
            i += 1
            buf = []
            while src[i] != '"':
                if src[i] == "\\":
                    e = src[i + 1]
                    if e == "\n":  # line continuation: skip leading whitespace
                        i += 2
                        while src[i] in " \t\n":
                            i += 1
                        continue
                    buf.append(RUST_ESCAPES.get(e, "\\" + e))
                    i += 2
                else:
                    buf.append(src[i])
                    i += 1
            out.append("".join(buf))
            i += 1
        elif c == "r" and src[i + 1] in '#"':
            m = re.match(r'r(#*)"', src[i:])
            hashes = m.group(1)
            start = i + len(m.group(0))
            end = src.index('"' + hashes, start)
            out.append(src[start:end])
            i = end + 1 + len(hashes)
        elif c == "(":
            depth += 1
            i += 1
        elif c == ")":
            if depth == 0:
                return out, i + 1
            depth -= 1
            i += 1
        else:
            i += 1
    raise ValueError("unterminated doc!(…)")


def manual_groups():
    """[(group title, [(name, sig, summary, example)])] in the manual's reading order."""
    src = (ROOT / "src" / "docs.rs").read_text(encoding="utf-8")
    groups = []
    for m in re.finditer(r"pub static (\w+): &\[Entry\] = &\[", src):
        table = m.group(1)
        if table not in TABLE_TITLES:
            continue
        end = src.index("\n];", m.end())
        body = src[m.end():end]
        title = TABLE_TITLES[table]
        current = None
        if title:
            current = (title, [])
            groups.append(current)
        i = 0
        while True:
            j = body.find("doc!(", i)
            k = body.find("// ──", i)
            if j < 0:
                break
            if 0 <= k < j:
                if title is None:
                    name = re.match(r"// ── (.+?) ─", body[k:]).group(1).strip()
                    current = (SECTION_TITLES.get(name, name.capitalize()), [])
                    groups.append(current)
                i = body.find("\n", k) + 1
                continue
            args, i = rust_strings(body, j + len("doc!("))
            example = args[3] if len(args) > 3 else ""
            current[1].append((args[0], args[1], args[2], example))
    return [g for g in groups if g[1]]


def prelude_entries():
    """[(name, kind, sig, comment, source)] from the prelude, in order."""
    src = (ROOT / "src" / "prelude.rs").read_text(encoding="utf-8")
    body = src[src.index('r#"') + 3: src.rindex('"#')]
    entries = []
    lines = body.split("\n")
    comment = []
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith(";;"):
            comment.append(line[2:].strip())
            i += 1
            continue
        m = re.match(r"\((defn|defmacro) (\S+)\s+\(([^)]*)\)", line)
        if m:
            form = [line]
            depth = line.count("(") - line.count(")")
            while depth > 0:
                i += 1
                form.append(lines[i])
                depth += lines[i].count("(") - lines[i].count(")")
            kind = "macro" if m.group(1) == "defmacro" else "function"
            params = m.group(3).strip()
            sig = "({} {})".format(m.group(2), params) if params else "({})".format(m.group(2))
            entries.append((m.group(2), kind, sig, " ".join(comment), "\n".join(form)))
        comment = []
        i += 1
    return entries


def build_reference():
    groups = manual_groups()
    prelude = prelude_entries()
    total = sum(len(g[1]) for g in groups) + len(prelude)
    out = [
        "# Function reference",
        "",
        "Everything callable from EELisp: {} entries. This page is generated by".format(total),
        "`scripts/build-docs.py` from the engine's built-in manual (`src/docs.rs`) and the",
        "prelude (`src/prelude.rs`), the same text `(source name)` prints in a REPL. Don't",
        "edit it by hand; change the manual or the prelude and rebuild.",
        "",
        "Signatures show the return type after `→`. Arguments written `:key …` are optional",
        "keyword arguments.",
        "",
        "## Index",
        "",
    ]
    sl = Slugger()
    for h in ["Function reference", "Index"]:
        sl.slug(h)
    anchors = {}
    for title, entries in groups:
        sl.slug(title)
        for e in entries:
            anchors[(title, e[0])] = sl.slug(e[0])
    sl.slug("Prelude")
    for e in prelude:
        anchors[("Prelude", e[0])] = sl.slug(e[0])
    for title, entries in groups:
        names = ", ".join("[`{}`](#{})".format(e[0], anchors[(title, e[0])]) for e in entries)
        out.append("- **{}**: {}".format(title, names))
    names = ", ".join("[`{}`](#{})".format(e[0], anchors[("Prelude", e[0])]) for e in prelude)
    out.append("- **Prelude**: {}".format(names))
    out.append("")
    for title, entries in groups:
        out += ["## " + title, ""]
        for name, sig, summary, example in entries:
            out += ["### " + name, "", "`{}`".format(sig), "", summary, ""]
            if example:
                out += ["```lisp", example, "```", ""]
    out += [
        "## Prelude",
        "",
        "Written in EELisp and loaded when the engine starts. Each definition is shown as written:",
        "read them as worked examples of the language.",
        "",
    ]
    for name, kind, sig, comment, source in prelude:
        out += ["### " + name, "", "`{}` · {}".format(sig, kind), "", comment, "",
                "```lisp", source, "```", ""]
    (DOCS / "reference.md").write_text("\n".join(out).rstrip() + "\n", encoding="utf-8")
    return total


# ── Markdown → HTML ───────────────────────────────────────────────────────────────────────────

class Slugger:
    """Heading anchors exactly as GitHub makes them: lower-case, punctuation dropped, spaces to
    hyphens, and -1, -2 … on repeats — so links work on GitHub and in the single page alike."""

    def __init__(self):
        self.seen = {}

    def slug(self, text):
        s = re.sub(r"`", "", text.strip().lower())
        s = re.sub(r"[^\w\- ]", "", s).replace(" ", "-")
        if s in self.seen:
            self.seen[s] += 1
            s = "{}-{}".format(s, self.seen[s])
        else:
            self.seen[s] = 0
        return s


def inline(text, chapter):
    """Inline Markdown: code, images, links, bold, italic."""
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for p in parts:
        if p.startswith("`") and p.endswith("`") and len(p) > 1:
            out.append("<code>" + html.escape(p[1:-1]) + "</code>")
            continue
        p = html.escape(p, quote=False)
        p = re.sub(r"!\[([^\]]*)\]\(([^)]+)\)", lambda m: image(m.group(1), m.group(2)), p)
        p = re.sub(r"\[([^\]]+)\]\(([^)]+)\)",
                   lambda m: '<a href="{}">{}</a>'.format(link(m.group(2), chapter), m.group(1)), p)
        p = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", p)
        p = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"<em>\1</em>", p)
        out.append(p)
    return "".join(out)


def inline_cell(text, chapter):
    # code spans inside a code span's `|` are fine: tables in the chapters don't escape pipes
    return inline(text, chapter)


def chapter_id(fname):
    return "top" if fname == "README.md" else fname[:-3]


def link(target, chapter):
    if re.match(r"https?://", target):
        return html.escape(target)
    path, _, frag = target.partition("#")
    if not path:
        return "#" + chapter_id(chapter) + "-" + frag
    if path.endswith(".md") and path in CHAPTERS:
        cid = chapter_id(path)
        return "#" + (cid + "-" + frag if frag else cid)
    if path == "eelisp.html":
        return "#top"
    return html.escape(target)


def image(alt, src):
    path = DOCS / src
    if path.suffix == ".svg" and path.exists():
        svg = path.read_text(encoding="utf-8")
        svg = re.sub(r"<style>.*?</style>", "", svg, flags=re.S)  # the page styles .ar-* itself
        svg = re.sub(r"<\?xml[^>]*>", "", svg)
        return '<figure class="diagram wide">{}</figure>'.format(svg.strip())
    return '<img alt="{}" src="{}">'.format(html.escape(alt), html.escape(src))


SPECIAL = {"def", "defn", "defun", "fn", "lambda", "let", "set!", "if", "cond", "and", "or", "do",
           "begin", "for-each", "loop", "recur", "quote", "quasiquote", "defmacro", "when",
           "unless", "deftable", "defform", "defrule", "defview", "defcategory", "deftemplate"}


def highlight_lisp(code):
    out = []
    i = 0
    n = len(code)
    prev = ""
    while i < n:
        c = code[i]
        if c == ";":
            j = code.find("\n", i)
            j = n if j < 0 else j
            out.append('<span class="c">' + html.escape(code[i:j]) + "</span>")
            i = j
        elif c == '"':
            j = i + 1
            while j < n and code[j] != '"':
                j += 2 if code[j] == "\\" else 1
            j = min(j + 1, n)
            out.append('<span class="s">' + html.escape(code[i:j]) + "</span>")
            i = j
        elif c == "→":
            j = code.find("\n", i)
            j = n if j < 0 else j
            out.append('<span class="r">' + html.escape(code[i:j]) + "</span>")
            i = j
        elif c in "()[]{}'`,@":
            out.append('<span class="p">' + html.escape(c) + "</span>")
            prev = c
            i += 1
            continue
        elif c.isspace():
            out.append(c)
            i += 1
            continue
        else:
            j = i
            while j < n and not code[j].isspace() and code[j] not in '()[]{}";':
                j += 1
            tok = code[i:j]
            esc = html.escape(tok)
            if tok.startswith(":"):
                out.append('<span class="k">' + esc + "</span>")
            elif re.fullmatch(r"-?\d+(\.\d+)?(e\d+)?%?", tok):
                out.append('<span class="n">' + esc + "</span>")
            elif prev == "(" and tok in SPECIAL:
                out.append('<span class="f">' + esc + "</span>")
            elif tok in ("true", "false", "nil"):
                out.append('<span class="n">' + esc + "</span>")
            else:
                out.append(esc)
            i = j
        prev = ""
    return "".join(out)


def md_to_html(md, chapter, toc):
    lines = md.split("\n")
    out = []
    i = 0
    para = []
    slugger = Slugger()

    def flush():
        if para:
            out.append("<p>" + inline(" ".join(para), chapter) + "</p>")
            para.clear()

    while i < len(lines):
        line = lines[i]
        fence = re.match(r"^```(\w*)\s*$", line)
        if fence:
            flush()
            lang = fence.group(1)
            j = i + 1
            while not lines[j].startswith("```"):
                j += 1
            code = "\n".join(lines[i + 1:j])
            if lang == "mermaid":
                out.append('<figure class="diagram">' + sequence_svg(code) + "</figure>")
            elif lang == "lisp":
                out.append('<pre class="lisp"><code>' + highlight_lisp(code) + "</code></pre>")
            else:
                out.append('<pre class="{}"><code>{}</code></pre>'.format(lang or "text", html.escape(code)))
            i = j + 1
            continue
        h = re.match(r"^(#{1,4}) (.+)$", line)
        if h:
            flush()
            level = len(h.group(1))
            text = h.group(2)
            cid = chapter_id(chapter)
            anchor = slugger.slug(text)
            hid = cid if level == 1 else cid + "-" + anchor
            out.append('<h{0} id="{1}">{2}<a class="anchor" href="#{1}" aria-label="Link to this section">#</a></h{0}>'
                       .format(level, hid, inline(text, chapter)))
            if level <= 2 or chapter == "reference.md":
                toc.append((level, hid, re.sub(r"`", "", text)))
            i += 1
            continue
        if line.startswith("|") and i + 1 < len(lines) and re.match(r"^\|[\s:|-]+\|\s*$", lines[i + 1]):
            flush()
            head = [c.strip() for c in line.strip().strip("|").split("|")]
            j = i + 2
            rows = []
            while j < len(lines) and lines[j].startswith("|"):
                rows.append([c.strip() for c in split_row(lines[j])])
                j += 1
            t = ['<div class="table"><table><thead><tr>']
            t += ["<th>" + inline_cell(c, chapter) + "</th>" for c in head]
            t.append("</tr></thead><tbody>")
            for r in rows:
                t.append("<tr>" + "".join("<td>" + inline_cell(c, chapter) + "</td>" for c in r) + "</tr>")
            t.append("</tbody></table></div>")
            out.append("".join(t))
            i = j
            continue
        li = re.match(r"^(\s*)([-*]|\d+\.) (.+)$", line)
        if li:
            flush()
            ordered = li.group(2)[0].isdigit()
            tag = "ol" if ordered else "ul"
            items = []
            while i < len(lines):
                m = re.match(r"^(\s*)([-*]|\d+\.) (.+)$", lines[i])
                if m:
                    items.append(m.group(3))
                elif lines[i].startswith("  ") and lines[i].strip() and items:
                    items[-1] += " " + lines[i].strip()
                else:
                    break
                i += 1
            out.append("<{}>{}</{}>".format(tag, "".join("<li>" + inline(x, chapter) + "</li>" for x in items), tag))
            continue
        if line.startswith(">"):
            flush()
            quote = []
            while i < len(lines) and lines[i].startswith(">"):
                quote.append(lines[i][1:].strip())
                i += 1
            out.append("<blockquote><p>" + inline(" ".join(quote), chapter) + "</p></blockquote>")
            continue
        if not line.strip():
            flush()
        elif line.startswith("![") and line.endswith(")"):
            flush()
            out.append(inline(line, chapter))
        else:
            para.append(line.strip())
        i += 1
    flush()
    return "\n".join(out)


def split_row(line):
    """Split a table row on | outside code spans."""
    cells, buf, in_code = [], [], False
    for c in line.strip().strip("|"):
        if c == "`":
            in_code = not in_code
        if c == "|" and not in_code:
            cells.append("".join(buf))
            buf = []
        else:
            buf.append(c)
    cells.append("".join(buf))
    return cells


# ── Mermaid sequence diagrams → SVG ───────────────────────────────────────────────────────────

CHAR_W = 6.6       # average glyph width at 12.5px
LINE_H = 16


def wrap(text, width):
    max_chars = max(8, int(width / CHAR_W))
    words, lines, cur = text.split(), [], ""
    for w in words:
        if cur and len(cur) + 1 + len(w) > max_chars:
            lines.append(cur)
            cur = w
        else:
            cur = (cur + " " + w).strip()
    if cur:
        lines.append(cur)
    return lines or [""]


def sequence_svg(src):
    parts, labels, events = [], {}, []
    for raw in src.split("\n"):
        line = raw.strip()
        if not line or line == "sequenceDiagram":
            continue
        m = re.match(r"participant (\w+)(?: as (.+))?$", line)
        if m:
            parts.append(m.group(1))
            labels[m.group(1)] = (m.group(2) or m.group(1)).strip()
            continue
        m = re.match(r"(\w+)\s*(-->>|->>)\s*(\w+)\s*:\s*(.*)$", line)
        if m:
            events.append(("msg", m.group(1), m.group(3), m.group(2) == "-->>", m.group(4)))
            continue
        m = re.match(r"Note (over|right of|left of) ([\w, ]+):\s*(.*)$", line)
        if m:
            who = [w.strip() for w in m.group(2).split(",")]
            events.append(("note", m.group(1), who, m.group(3)))
            continue
        m = re.match(r"(loop|alt|opt|else)\b\s*(.*)$", line)
        if m:
            events.append((m.group(1), m.group(2)))
            continue
        if line == "end":
            events.append(("end",))
            continue
        raise ValueError("unsupported mermaid line: " + line)

    col = {p: k for k, p in enumerate(parts)}
    box_w = max(118, max(len(labels[p]) for p in parts) * 7.2 + 24)
    gap = max(box_w + 26, 150)
    for e in events:
        if e[0] == "msg" and e[1] != e[2]:
            dist = abs(col[e[1]] - col[e[2]])
            need = (min(len(e[4]), 44) * CHAR_W + 24) / dist
            gap = max(gap, min(need, 250))
    margin = box_w / 2 + 24
    xs = {p: margin + col[p] * gap for p in parts}
    width = margin * 2 + (len(parts) - 1) * gap
    self_room = 0
    for e in events:
        if e[0] == "msg" and e[1] == e[2] and col[e[1]] == len(parts) - 1:
            self_room = max(self_room, 170)
    width += self_room

    body, y, stack = [], 70.0, []
    left_edge, right_edge = 10, width - 10

    for e in events:
        kind = e[0]
        if kind == "msg":
            _, a, b, dashed, text = e
            cls = "sq-msg dashed" if dashed else "sq-msg"
            if a == b:
                x = xs[a]
                avail = (gap - 40) if col[a] < len(parts) - 1 else self_room - 20
                lines = wrap(text, avail)
                y += 6
                for k, t in enumerate(lines):
                    body.append('<text class="sq-t" x="{:.0f}" y="{:.0f}">{}</text>'.format(x + 14, y + k * LINE_H + 12, html.escape(t)))
                top = y + len(lines) * LINE_H + 8
                body.append('<path class="{}" d="M{:.0f} {:.0f} h32 v14 h-30" marker-end="url(#sq-arrow)"/>'.format(cls, x, top))
                y = top + 28
            else:
                x1, x2 = xs[a], xs[b]
                lines = wrap(text, abs(x2 - x1) - 20)
                y += 4
                mid = (x1 + x2) / 2
                for k, t in enumerate(lines):
                    body.append('<text class="sq-t" x="{:.0f}" y="{:.0f}" text-anchor="middle">{}</text>'.format(mid, y + k * LINE_H + 12, html.escape(t)))
                ly = y + len(lines) * LINE_H + 4
                end = x2 - 3 if x2 > x1 else x2 + 3
                body.append('<line class="{}" x1="{:.0f}" y1="{:.0f}" x2="{:.0f}" y2="{:.0f}" marker-end="url(#sq-arrow)"/>'.format(cls, x1, ly, end, ly))
                y = ly + 16
        elif kind == "note":
            _, where, who, text = e
            if where == "over":
                xa, xb = min(xs[w] for w in who), max(xs[w] for w in who)
                if xa == xb:
                    w_ = min(gap * 1.6, 240)
                    x0, x1 = xa - w_ / 2, xa + w_ / 2
                else:
                    x0, x1 = xa - 50, xb + 50
            elif where == "right of":
                x0 = xs[who[0]] + 10
                x1 = x0 + min(gap * 1.4, 230)
            else:
                x1 = xs[who[0]] - 10
                x0 = x1 - min(gap * 1.4, 230)
            x0, x1 = max(x0, left_edge + 4), min(x1, right_edge - 4)
            lines = wrap(text, x1 - x0 - 16)
            h = len(lines) * LINE_H + 12
            y += 4
            body.append('<rect class="sq-note" x="{:.0f}" y="{:.0f}" width="{:.0f}" height="{:.0f}" rx="4"/>'.format(x0, y, x1 - x0, h))
            for k, t in enumerate(lines):
                body.append('<text class="sq-nt" x="{:.0f}" y="{:.0f}" text-anchor="middle">{}</text>'.format((x0 + x1) / 2, y + 18 + k * LINE_H, html.escape(t)))
            y += h + 12
        elif kind in ("loop", "alt", "opt"):
            depth = len(stack)
            stack.append({"kind": kind, "y": y, "label": e[1], "depth": depth, "elses": []})
            y += 30
        elif kind == "else":
            stack[-1]["elses"].append((y, e[1]))
            y += 26
        elif kind == "end":
            blk = stack.pop()
            inset = 12 + blk["depth"] * 9
            x0, x1 = left_edge + inset, right_edge - inset
            y0, y1 = blk["y"], y + 4
            tag = blk["kind"]
            tw = len(tag) * 7 + 16
            body.insert(0, '<rect class="sq-block" x="{:.0f}" y="{:.0f}" width="{:.0f}" height="{:.0f}" rx="4"/>'.format(x0, y0, x1 - x0, y1 - y0))
            body.append('<path class="sq-tag" d="M{0:.0f} {1:.0f} h{2:.0f} v14 l-7 7 h-{3:.0f} z"/>'.format(x0, y0, tw, tw - 7))
            body.append('<text class="sq-tagt" x="{:.0f}" y="{:.0f}">{}</text>'.format(x0 + 8, y0 + 15, tag))
            if blk["label"]:
                body.append('<text class="sq-bl" x="{:.0f}" y="{:.0f}">[{}]</text>'.format(x0 + tw + 8, y0 + 15, html.escape(blk["label"])))
            for ey, label in blk["elses"]:
                body.append('<line class="sq-else" x1="{:.0f}" y1="{:.0f}" x2="{:.0f}" y2="{:.0f}"/>'.format(x0, ey + 2, x1, ey + 2))
                body.append('<text class="sq-bl" x="{:.0f}" y="{:.0f}">[{}]</text>'.format(x0 + 8, ey + 18, html.escape(label)))
            y = y1 + 10

    y += 10
    height = y + 54
    head = []
    for p in parts:
        x = xs[p]
        head.append('<line class="sq-life" x1="{0:.0f}" y1="52" x2="{0:.0f}" y2="{1:.0f}"/>'.format(x, y))
        for top in (12, y):
            head.append('<rect class="sq-actor" x="{:.0f}" y="{:.0f}" width="{:.0f}" height="38" rx="7"/>'.format(x - box_w / 2, top, box_w))
            head.append('<text class="sq-at" x="{:.0f}" y="{:.0f}" text-anchor="middle">{}</text>'.format(x, top + 24, html.escape(labels[p])))
    title = "Sequence diagram: " + ", ".join(labels[p] for p in parts)
    return ('<svg class="seq" xmlns="http://www.w3.org/2000/svg" style="max-width:{w:.0f}px" viewBox="0 0 {w:.0f} {h:.0f}" role="img" aria-label="{t}">'
            '<defs><marker id="sq-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">'
            '<path class="sq-ah" d="M0,0 L10,5 L0,10 z"/></marker></defs>{head}{body}</svg>'
            ).format(w=width, h=height, t=html.escape(title), head="".join(head), body="".join(body))


# ── the page ──────────────────────────────────────────────────────────────────────────────────

CSS = r"""
:root {
  --bg: #fcfbf9; --bg-alt: #f4f2ee; --fg: #1b1a17; --muted: #6b675e; --line: #e3dfd7;
  --card: #ffffff; --accent: #1f6f5c; --accent-soft: #e8f2ef; --code-bg: #f6f4f0; --code-fg: #2a2823;
  --c-comment: #8a857a; --c-string: #9a4a1c; --c-key: #1f6f5c; --c-num: #6a3fa0; --c-form: #1d4f91;
  --radius: 10px;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #14130f; --bg-alt: #1b1a15; --fg: #ece8e0; --muted: #a09a8d; --line: #302d26;
    --card: #1e1c17; --accent: #5cc4a7; --accent-soft: #16241f; --code-bg: #1e1c17; --code-fg: #ddd8ce;
    --c-comment: #8d887c; --c-string: #e0a071; --c-key: #5cc4a7; --c-num: #c3a3f0; --c-form: #8cb8f5;
  }
}
:root[data-theme="dark"] {
  --bg: #14130f; --bg-alt: #1b1a15; --fg: #ece8e0; --muted: #a09a8d; --line: #302d26;
  --card: #1e1c17; --accent: #5cc4a7; --accent-soft: #16241f; --code-bg: #1e1c17; --code-fg: #ddd8ce;
  --c-comment: #8d887c; --c-string: #e0a071; --c-key: #5cc4a7; --c-num: #c3a3f0; --c-form: #8cb8f5;
}
*, *::before, *::after { box-sizing: border-box; }
html { scroll-behavior: smooth; scroll-padding-top: 72px; -webkit-text-size-adjust: 100%; }
body { margin: 0; background: var(--bg); color: var(--fg);
  font: 16.5px/1.65 ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased; }
a { color: var(--accent); text-underline-offset: 2px; }
code, pre { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; }
header.bar { position: sticky; top: 0; z-index: 20; display: flex; align-items: center; gap: 12px;
  padding: 10px 16px; border-bottom: 1px solid var(--line);
  background: color-mix(in srgb, var(--bg) 90%, transparent); backdrop-filter: blur(10px); }
.brand { font-weight: 680; letter-spacing: -.02em; color: var(--fg); text-decoration: none; font-size: 1.05rem; }
.brand .lam { color: var(--accent); }
.bar .spacer { flex: 1; }
.site-links { display: flex; gap: 18px; margin-right: 8px; }
.site-links a { color: var(--muted); text-decoration: none; font-size: .92rem; }
.site-links a:hover, .site-links a[aria-current="page"] { color: var(--fg); }
.bar button { font: inherit; font-size: .88rem; color: var(--fg); background: var(--card);
  border: 1px solid var(--line); border-radius: 8px; padding: 5px 10px; cursor: pointer; }
.bar button:hover { border-color: var(--accent); }
#menu-btn { display: none; }
.layout { display: grid; grid-template-columns: 250px minmax(0, 1fr); max-width: 1240px; margin: 0 auto; }
nav.toc { position: sticky; top: 53px; align-self: start; max-height: calc(100vh - 53px); overflow-y: auto;
  padding: 24px 12px 40px 20px; border-right: 1px solid var(--line); font-size: .9rem; }
nav.toc input { width: 100%; font: inherit; padding: 7px 10px; border-radius: 8px; border: 1px solid var(--line);
  background: var(--card); color: var(--fg); margin-bottom: 14px; }
nav.toc input:focus { outline: 2px solid var(--accent); outline-offset: 1px; }
nav.toc ol { list-style: none; margin: 0; padding: 0; }
nav.toc li { margin: 0; }
nav.toc a { display: block; padding: 3px 8px; border-radius: 6px; color: var(--muted); text-decoration: none; }
nav.toc a:hover { color: var(--fg); background: var(--bg-alt); }
nav.toc a.active { color: var(--accent); background: var(--accent-soft); font-weight: 600; }
nav.toc .l2 a { padding-left: 20px; font-size: .84rem; }
nav.toc li.fn { display: none; }
nav.toc li.fn a { padding-left: 20px; font-size: .84rem; font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; }
nav.toc .hit-none { color: var(--muted); padding: 4px 8px; font-size: .84rem; }
main { padding: 32px 40px 96px; min-width: 0; }
main > section { max-width: 780px; padding-bottom: 40px; margin-bottom: 40px; border-bottom: 1px solid var(--line); }
main > section:last-child { border-bottom: 0; }
h1, h2, h3, h4 { line-height: 1.25; letter-spacing: -.02em; font-weight: 660; position: relative; }
h1 { font-size: 2rem; margin: .2em 0 .6em; }
h2 { font-size: 1.45rem; margin: 2em 0 .6em; }
h3 { font-size: 1.12rem; margin: 1.8em 0 .5em; }
h4 { font-size: 1rem; margin: 1.5em 0 .4em; }
.anchor { margin-left: .4em; color: var(--line); text-decoration: none; font-weight: 400; opacity: 0; }
h1:hover .anchor, h2:hover .anchor, h3:hover .anchor, h4:hover .anchor, .anchor:focus { opacity: 1; color: var(--muted); }
p, ul, ol { margin: 0 0 1em; }
ul, ol { padding-left: 1.3em; }
li { margin: .25em 0; }
:not(pre) > code { background: var(--code-bg); color: var(--code-fg); padding: .1em .35em; border-radius: 5px;
  font-size: .88em; overflow-wrap: anywhere; }
pre { background: var(--code-bg); color: var(--code-fg); border: 1px solid var(--line); border-radius: var(--radius);
  padding: 14px 16px; overflow-x: auto; font-size: .86rem; line-height: 1.55; margin: 0 0 1.2em; tab-size: 2; }
pre .c { color: var(--c-comment); font-style: italic; }
pre .s { color: var(--c-string); }
pre .k { color: var(--c-key); }
pre .n { color: var(--c-num); }
pre .f { color: var(--c-form); font-weight: 600; }
pre .p { color: var(--muted); }
pre .r { color: var(--accent); }
.table { overflow-x: auto; margin: 0 0 1.2em; border: 1px solid var(--line); border-radius: var(--radius); }
table { border-collapse: collapse; width: 100%; font-size: .9rem; }
th, td { text-align: left; vertical-align: top; padding: 8px 12px; border-bottom: 1px solid var(--line); }
th { background: var(--bg-alt); font-weight: 620; }
tr:last-child td { border-bottom: 0; }
blockquote { margin: 0 0 1em; padding: 4px 16px; border-left: 3px solid var(--accent); color: var(--muted); }
figure.diagram { margin: 8px 0 24px; padding: 12px; background: var(--card); border: 1px solid var(--line);
  border-radius: var(--radius); overflow-x: auto; }
figure.diagram svg.seq { width: 100%; min-width: 540px; height: auto; }
figure.diagram svg { display: block; margin: 0 auto; font-family: ui-sans-serif, -apple-system, "Segoe UI", Roboto, sans-serif; }
figure.diagram.wide svg { width: 100%; min-width: 640px; height: auto; }
.sq-actor { fill: var(--accent-soft); stroke: var(--accent); stroke-width: 1.2; }
.sq-at { fill: var(--fg); font-size: 13px; font-weight: 620; }
.sq-life { stroke: var(--line); stroke-width: 1.4; stroke-dasharray: 4 4; }
.sq-msg { stroke: var(--fg); stroke-width: 1.3; fill: none; }
.sq-msg.dashed { stroke-dasharray: 5 4; stroke: var(--muted); }
.sq-ah { fill: var(--fg); }
.sq-t { fill: var(--fg); font-size: 12.5px; }
.sq-note { fill: var(--bg-alt); stroke: var(--line); }
.sq-nt { fill: var(--muted); font-size: 12px; font-style: italic; }
.sq-block { fill: none; stroke: var(--muted); stroke-width: 1; stroke-dasharray: 2 3; }
.sq-tag { fill: var(--bg-alt); stroke: var(--muted); stroke-width: 1; }
.sq-tagt { fill: var(--fg); font-size: 11.5px; font-weight: 700; }
.sq-bl { fill: var(--muted); font-size: 12px; font-weight: 600; }
.sq-else { stroke: var(--muted); stroke-width: 1; stroke-dasharray: 5 4; }
.ar-bg { fill: var(--card); }
.ar-band { fill: var(--bg-alt); stroke: var(--line); }
.ar-box { fill: var(--card); stroke: var(--line); }
.ar-key { fill: var(--accent-soft); stroke: var(--accent); }
.ar-t { fill: var(--fg); font-size: 13px; font-weight: 600; }
.ar-s { fill: var(--muted); font-size: 11px; }
.ar-l { fill: var(--accent); font-size: 11px; font-weight: 700; letter-spacing: .08em; }
.ar-a { stroke: var(--muted); stroke-width: 1.4; fill: none; }
.ar-ah { fill: var(--muted); }
#reference h3 { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 1rem;
  margin-top: 1.6em; padding-top: 1.1em; border-top: 1px dashed var(--line); }
footer { color: var(--muted); font-size: .85rem; padding: 0 40px 40px; max-width: 1240px; margin: 0 auto; }
@media (max-width: 900px) {
  #menu-btn { display: inline-block; }
  .site-links { display: none; }
  .layout { display: block; }
  nav.toc { display: none; position: fixed; top: 53px; left: 0; right: 0; bottom: 0; max-height: none;
    background: var(--bg); z-index: 15; border-right: 0; padding: 16px; }
  body.menu-open nav.toc { display: block; }
  main { padding: 20px 16px 72px; }
  footer { padding: 0 16px 32px; }
  h1 { font-size: 1.65rem; }
}
@media print {
  header.bar, nav.toc { display: none; }
  .layout { display: block; }
  pre, figure { break-inside: avoid; }
}
"""

JS = r"""
(function () {
  var root = document.documentElement;
  var btn = document.getElementById("theme-btn");
  function stored() { try { return localStorage.getItem("eelisp-docs-theme"); } catch (e) { return null; } }
  function store(v) { try { localStorage.setItem("eelisp-docs-theme", v); } catch (e) {} }
  function current() {
    var t = root.getAttribute("data-theme");
    if (t) return t;
    return window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  function label() { btn.textContent = current() === "dark" ? "Light" : "Dark"; }
  var s = stored();
  if (s === "dark" || s === "light") root.setAttribute("data-theme", s);
  label();
  btn.addEventListener("click", function () {
    var next = current() === "dark" ? "light" : "dark";
    root.setAttribute("data-theme", next); store(next); label();
  });

  var menu = document.getElementById("menu-btn");
  menu.addEventListener("click", function () {
    var open = document.body.classList.toggle("menu-open");
    menu.setAttribute("aria-expanded", open ? "true" : "false");
  });
  var toc = document.querySelector("nav.toc");
  toc.addEventListener("click", function (e) {
    if (e.target.tagName === "A") { document.body.classList.remove("menu-open"); menu.setAttribute("aria-expanded", "false"); }
  });

  var filter = document.getElementById("toc-filter");
  var entries = [].slice.call(toc.querySelectorAll("li"));
  var none = document.getElementById("toc-none");
  filter.addEventListener("input", function () {
    var q = filter.value.trim().toLowerCase(), shown = 0;
    entries.forEach(function (li) {
      var hit = !q || li.textContent.toLowerCase().indexOf(q) >= 0;
      li.style.display = hit ? "" : "none"; if (hit) shown++;
    });
    none.style.display = shown ? "none" : "block";
  });
  filter.addEventListener("keydown", function (e) {
    if (e.key !== "Enter") return;
    var first = entries.filter(function (li) { return li.style.display !== "none"; })[0];
    if (first) { location.hash = first.querySelector("a").getAttribute("href"); document.body.classList.remove("menu-open"); }
  });

  var links = {};
  [].forEach.call(toc.querySelectorAll("a"), function (a) { links[a.getAttribute("href").slice(1)] = a; });
  var heads = [].slice.call(document.querySelectorAll("main h1[id], main h2[id], main h3[id]"))
    .filter(function (h) { return links[h.id]; });
  var active = null;
  function spy() {
    var y = window.scrollY + 90, cur = heads[0];
    for (var i = 0; i < heads.length; i++) { if (heads[i].offsetTop <= y) cur = heads[i]; else break; }
    if (cur && links[cur.id] !== active) {
      if (active) active.classList.remove("active");
      active = links[cur.id]; active.classList.add("active");
    }
  }
  window.addEventListener("scroll", spy, { passive: true });
  spy();
})();
"""


SITE_LINKS = [
    ("guide.html", "Guide"),
    ("reference.html", "Reference"),
    ("docs.html", "Docs"),
    ("embed.html", "Embedding"),
    ("https://github.com/santacroce-tech/eelisp-rs", "Source"),
    ("https://eeditor.app", "EEditor"),
]

FAVICON = ("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect "
           "width='64' height='64' rx='14' fill='%231f6f5c'/%3E%3Ctext x='32' y='46' font-size='42' "
           "text-anchor='middle' fill='white' font-family='Georgia,serif'%3E%CE%BB%3C/text%3E%3C/svg%3E")


def build_html(target, site=False):
    sections, toc = [], []
    for fname in CHAPTERS:
        md = (DOCS / fname).read_text(encoding="utf-8")
        if fname == "README.md":
            md = md.replace("# EELisp documentation", "# EELisp", 1)
        body = md_to_html(md, fname, toc)
        sections.append('<section id="sec-{}">{}</section>'.format(chapter_id(fname), body))
    items = []
    for level, hid, text in toc:
        cls = "l{}".format(min(level, 2)) + (" fn" if level == 3 else "")
        items.append('<li class="{}"><a href="#{}">{}</a></li>'.format(cls, hid, html.escape(text)))
    if site:
        head = ('<link rel="icon" type="image/svg+xml" href="assets/favicon.svg">\n'
                '<meta property="og:title" content="EELisp documentation">\n'
                '<meta property="og:description" content="The whole language on one page: syntax, the database, '
                'the agenda, sheets, architecture, embedding and every function.">\n'
                '<meta property="og:type" content="website">\n'
                '<meta property="og:url" content="https://eelisp.app/docs.html">')
        brand_href = "/"
        links = '<nav class="site-links" aria-label="Site">{}</nav>'.format("".join(
            '<a href="{}"{}>{}</a>'.format(h, ' aria-current="page"' if h == "docs.html" else "", t)
            for h, t in SITE_LINKS))
        footer = ('Generated from <code>docs/</code> in the '
                  '<a href="https://github.com/santacroce-tech/eelisp-rs">source repository</a>. '
                  'EEditor and EELisp are free software under the '
                  '<a href="https://github.com/santacroce-tech/eelisp-rs/blob/main/LICENSE">MIT licence</a>. '
                  '© 2026 Roberto Santacroce Martins.')
    else:
        head = '<link rel="icon" href="{}">'.format(FAVICON)
        brand_href = "#top"
        links = ""
        footer = ('Generated from <code>docs/*.md</code> by <code>scripts/build-docs.py</code>. '
                  'EELisp is MIT licensed.')
    page = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>EELisp Documentation</title>
<meta name="description" content="The EELisp language: syntax, the database, the agenda, sheets, architecture, embedding and the full function reference.">
{head}
<style>{css}</style>
</head>
<body>
<header class="bar">
  <button id="menu-btn" aria-controls="toc" aria-expanded="false">Contents</button>
  <a class="brand" href="{brand_href}"><span class="lam">λ</span> EELisp</a>
  <span class="spacer"></span>{links}
  <button id="theme-btn" type="button" aria-label="Switch colour theme">Dark</button>
</header>
<div class="layout">
  <nav class="toc" id="toc" aria-label="Contents">
    <input id="toc-filter" type="search" placeholder="Find a section or function…" aria-label="Find a section or function">
    <ol>{toc}</ol>
    <div id="toc-none" class="hit-none" style="display:none">No section matches.</div>
  </nav>
  <main>
{sections}
  </main>
</div>
<footer>{footer}</footer>
<script>{js}</script>
</body>
</html>
""".format(css=CSS, toc="".join(items), sections="\n".join(sections), js=JS,
           head=head, brand_href=brand_href, links=links, footer=footer)
    target.write_text(page, encoding="utf-8")
    return len(page)


def main():
    n = build_reference()
    print("docs/reference.md: {} entries".format(n))
    size = build_html(DOCS / "eelisp.html")
    print("docs/eelisp.html: {:,} bytes".format(size))
    size = build_html(ROOT / "site" / "docs.html", site=True)
    print("site/docs.html: {:,} bytes".format(size))


if __name__ == "__main__":
    sys.exit(main())
