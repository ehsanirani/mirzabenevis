// Minimal Markdown for answers that are read, not pasted: headings, `-`/`*`/`1.` lists (no
// nesting), **bold**, `code`, fenced code and tables; everything else stays plain text.
// `md(src)` returns DOM nodes built with createElement + text nodes only, so nothing in the
// answer is ever parsed as HTML. Plain script (global `md`); `node ui/md.js` runs the self-check.
function domNode(tag, kids) {
  const el = document.createElement(tag);
  el.append(...kids); // strings become text nodes
  return el;
}

function md(src, h = domNode) {
  const inline = (s) => s.split(/(\*\*[^*\n]+\*\*|`[^`\n]+`)/)
    .map((p, i) => (i % 2 === 0 ? p : p[0] === "`" ? h("code", [p.slice(1, -1)]) : h("strong", [p.slice(2, -2)])))
    .filter((p) => p !== "");
  const cells = (l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => inline(c.trim()));
  const isRule = (l) => /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(l || "");
  const item = /^\s*(?:([-*])|\d+[.)])\s+(.*)$/;
  const fence = /^\s*```/;
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
      const items = [];
      for (; i < lines.length; i++) {
        // A blank line between two items of the same list does not end it.
        const next = item.exec(lines[i + 1] || "");
        if (!lines[i].trim() && next && !!next[1] === bullet) continue;
        if (!(m = item.exec(lines[i])) || !!m[1] !== bullet) break;
        items.push(h("li", inline(m[2])));
      }
      i--;
      out.push(h(bullet ? "ul" : "ol", items));
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
  const html = (n) => (typeof n === "string" ? esc(n) : `<${n.tag}>${n.kids.map(html).join("")}</${n.tag}>`);
  const r = (s) => md(s, (tag, kids) => ({ tag, kids })).map(html).join("");
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
  console.log("md.js ok");
}
