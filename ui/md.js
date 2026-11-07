// Minimal Markdown for answers that are read, not pasted: headings, `-`/`*`/`1.` lists (nested
// by indentation), > quotes, **bold**, *italic*/_italic_, `code`, https links, fenced code and
// tables; everything else stays plain text. `md(src)` returns DOM nodes built with createElement
// + text nodes only, so nothing in the answer is ever parsed as HTML. Links get an href but main.js
// opens them in the browser; the popup never navigates. Plain script (global `md`); `node ui/md.js`
// runs the self-check.
// Only top-level blocks pick a direction: a list, table or quote takes one for all it holds
// (dir="auto" skips children that have their own dir, so nested blocks lose theirs).
const AUTO_DIR = new Set(["p", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6", "table", "blockquote"]);
function domNode(tag, kids, attrs) {
  const el = Object.assign(document.createElement(tag), attrs);
  if (AUTO_DIR.has(tag)) el.dir = "auto"; // a Persian paragraph, list or table reads right to left
  kids.forEach((k) => k.removeAttribute?.("dir"));
  el.append(...kids); // strings become text nodes
  return el;
}

// One capture group (split keeps it): code, bold, [link](url), *em*, _em_, a bare https URL.
// Emphasis needs a non-letter, non-digit outside its markers (any script), so 2*3*4 and
// snake_case_names stay text. URLs may hold one level of (parens); a bare one drops trailing punctuation.
const W = "[\\p{L}\\p{N}_]";
const PAREN = "\\([^\\s<>()]*\\)";
const INLINE = new RegExp("(" + [
  "`[^`\\n]+`",
  "\\*\\*[^*\\n]+\\*\\*",
  `\\[[^[\\]\\n]+\\]\\(https://(?:[^()\\s]|${PAREN})+\\)`, // no [ in the text: linear on "[[[["
  `(?<!${W}|\\*)\\*[^*\\s](?:[^*\\n]*[^*\\s])?\\*(?!${W}|\\*)`,
  `(?<!${W})_[^_\\s](?:[^_\\n]*[^_\\s])?_(?!${W})`,
  `https://(?:[^\\s<>()]|${PAREN})*(?:[^\\s<>().,;:!?'"]|${PAREN})`,
].join("|") + ")", "u");
const indent = (l) => l.match(/^\s*/)[0].length;

function md(src, h = domNode) {
  const link = (url, kids) => h("a", kids, { href: url, title: url });
  const inline = (s) => s.split(INLINE).map((p, i) => {
    if (i % 2 === 0) return p;
    if (p[0] === "`") return h("code", [p.slice(1, -1)]);
    if (p.startsWith("**")) return h("strong", inline(p.slice(2, -2)));
    if (p[0] === "*" || p[0] === "_") return h("em", inline(p.slice(1, -1)));
    if (p[0] === "[") return link(p.slice(p.indexOf("](") + 2, -1), inline(p.slice(1, p.indexOf("](")))); // text has no ]
    return link(p, [p]);
  }).filter((p) => p !== "");
  const cells = (l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => inline(c.trim()));
  const isRule = (l) => /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(l || "");
  const item = /^\s*(?:([-*])|\d+[.)])\s+(.*)$/;
  const fence = /^\s*```/;
  const quote = /^\s*>\s?/;
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  let para = [];
  const flush = () => {
    if (para.length) out.push(h("p", para.flatMap((l, i) => (i ? [h("br", []), ...inline(l)] : inline(l)))));
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    let m;
    if (fence.test(l)) {
      flush();
      const body = [];
      while (++i < lines.length && !fence.test(lines[i])) body.push(lines[i]);
      out.push(h("pre", [h("code", [body.join("\n")])]));
    } else if ((m = /^(#{1,6})\s+(.*?)(\s+#+)?\s*$/.exec(l))) {
      flush();
      out.push(h(`h${m[1].length}`, inline(m[2])));
    } else if (quote.test(l)) {
      flush();
      const body = [];
      for (; i < lines.length && quote.test(lines[i]); i++) body.push(lines[i].replace(quote, ""));
      i--;
      out.push(h("blockquote", md(body.join("\n"), h)));
    } else if (l.includes("|") && isRule(lines[i + 1])) {
      flush();
      const head = h("tr", cells(l).map((c) => h("th", c)));
      const rows = [];
      for (i += 2; i < lines.length && lines[i].includes("|"); i++) rows.push(h("tr", cells(lines[i]).map((c) => h("td", c))));
      i--;
      out.push(h("table", [h("thead", [head]), h("tbody", rows)]));
    } else if ((m = item.exec(l))) {
      flush();
      const bullet = !!m[1];
      const base = indent(l);
      const items = []; // the children of each <li>
      let sub = []; // lines indented under the current item: a nested list or more text
      const close = () => {
        if (!sub.length) return;
        const cut = Math.min(...sub.filter((s) => s.trim()).map(indent));
        items.at(-1).push(...md(sub.map((s) => s.slice(cut)).join("\n"), h));
        sub = [];
      };
      for (; i < lines.length; i++) {
        const cur = lines[i];
        const next = lines[i + 1] || "";
        if (!cur.trim()) {
          // A blank line goes on to more indented lines (and is kept there), or to the same list's next item.
          const n = item.exec(next);
          if (next.trim() && indent(next) > base) sub.push("");
          else if (!(n && indent(next) === base && !!n[1] === bullet)) break;
          continue;
        }
        if (indent(cur) > base) {
          sub.push(cur);
          continue;
        }
        if (!(m = item.exec(cur)) || !!m[1] !== bullet) break;
        close();
        items.push(inline(m[2]));
      }
      close();
      i--;
      out.push(h(bullet ? "ul" : "ol", items.map((k) => h("li", k))));
    } else if (l.trim()) {
      para.push(l.trim());
    } else {
      flush();
    }
  }
  flush();
  return out;
}

if (typeof module !== "undefined" && require.main === module) {
  const assert = require("assert");
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const attrs = (a) => Object.entries(a || {}).map(([k, v]) => ` ${k}="${esc(v)}"`).join("");
  const html = (n) => (typeof n === "string" ? esc(n) : `<${n.tag}${attrs(n.attrs)}>${n.kids.map(html).join("")}</${n.tag}>`);
  const r = (s) => md(s, (tag, kids, a) => ({ tag, kids, attrs: a })).map(html).join("");
  assert.strictEqual(r("# Title #\nsome **bold** and `a*b`\nnext line"),
    "<h1>Title</h1><p>some <strong>bold</strong> and <code>a*b</code><br></br>next line</p>");
  assert.strictEqual(r("- one\n* two\n\n- three\n1. first\n2) second"),
    "<ul><li>one</li><li>two</li><li>three</li></ul><ol><li>first</li><li>second</li></ol>");
  assert.strictEqual(r("**Bold** start\n\nnot*a list*"), "<p><strong>Bold</strong> start</p><p>not*a list*</p>");
  assert.strictEqual(r("| A | B |\n|---|:-:|\n| 1 | **2** |\n|3|\nafter"),
    "<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td><strong>2</strong></td></tr>"
    + "<tr><td>3</td></tr></tbody></table><p>after</p>");
  assert.strictEqual(r("```js\nlet x = 1;\n\n# not a heading\n```\ntail"),
    "<pre><code>let x = 1;\n\n# not a heading</code></pre><p>tail</p>");
  assert.strictEqual(r("```\nunclosed"), "<pre><code>unclosed</code></pre>");
  // Markup in the answer stays text.
  assert.strictEqual(r("<b>x</b> & <img src=x onerror=alert(1)>"), "<p>&lt;b>x&lt;/b> &amp; &lt;img src=x onerror=alert(1)></p>");
  assert.strictEqual(r("a | b"), "<p>a | b</p>");
  assert.strictEqual(r(""), "");
  // Emphasis, but not inside words or numbers.
  assert.strictEqual(r("*it* and _it_, 2*3*4, snake_case_name, a * b * c"),
    "<p><em>it</em> and <em>it</em>, 2*3*4, snake_case_name, a * b * c</p>");
  assert.strictEqual(r("**bold _em_ in**"), "<p><strong>bold <em>em</em> in</strong></p>");
  // https links only; a bare URL loses trailing punctuation; javascript: and http: stay text.
  assert.strictEqual(r("[the *docs*](https://x.org/a_(b)) see https://x.org/p?q=1. [x](javascript:alert(1)) http://y.org"),
    '<p><a href="https://x.org/a_(b)" title="https://x.org/a_(b)">the <em>docs</em></a> see '
    + '<a href="https://x.org/p?q=1" title="https://x.org/p?q=1">https://x.org/p?q=1</a>. [x](javascript:alert(1)) http://y.org</p>');
  assert.strictEqual(r("`[a](https://x.org)` and `*not em*`"), "<p><code>[a](https://x.org)</code> and <code>*not em*</code></p>");
  assert.strictEqual(r("۲*۳*۴ سلام_دنیا_خوب *تاکید*"), "<p>۲*۳*۴ سلام_دنیا_خوب <em>تاکید</em></p>");
  assert.strictEqual(r("https://en.wikipedia.org/wiki/Foo_(bar), (see https://x.org/a)"),
    '<p><a href="https://en.wikipedia.org/wiki/Foo_(bar)" title="https://en.wikipedia.org/wiki/Foo_(bar)">'
    + 'https://en.wikipedia.org/wiki/Foo_(bar)</a>, (see <a href="https://x.org/a" title="https://x.org/a">https://x.org/a</a>)</p>');
  // The URL starts at the first "](": no javascript: href hides behind a second one.
  assert.ok(!r("[a](https://x](javascript:alert`1`//))").includes('href="javascript'));
  // Quotes hold any Markdown.
  assert.strictEqual(r("> quoted **text**\n> - a\n>\n> more\nafter"),
    "<blockquote><p>quoted <strong>text</strong></p><ul><li>a</li></ul><p>more</p></blockquote><p>after</p>");
  // Nested lists by indentation, mixed types, blank lines inside.
  assert.strictEqual(r("- a\n  - a1\n  - a2\n\n    1. deep\n- b\n  more text\n1. one"),
    "<ul><li>a<ul><li>a1</li><li>a2<ol><li>deep</li></ol></li></ul></li><li>b<p>more text</p></li></ul><ol><li>one</li></ol>");
  // Blank lines under an item are kept (code blocks, paragraphs).
  assert.strictEqual(r("1. Install:\n   ```\n   npm i\n\n   npm run\n   ```\n2. Done\n   p1\n\n   p2"),
    "<ol><li>Install:<pre><code>npm i\n\nnpm run</code></pre></li><li>Done<p>p1</p><p>p2</p></li></ol>");
  console.log("md.js ok");
}
