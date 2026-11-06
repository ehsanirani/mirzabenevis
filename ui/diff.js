// Word-level diff: returns [op, text] runs with op "=", "-" (deleted) or "+" (inserted).
// Plain script (global `wordDiff` in the popup); `node ui/diff.js` runs the self-check.
function wordDiff(a, b) {
  const A = a.split(/(\s+)/).filter(Boolean);
  const B = b.split(/(\s+)/).filter(Boolean);
  const out = [];
  const push = (op, t) => {
    const last = out[out.length - 1];
    if (last && last[0] === op) last[1] += t;
    else out.push([op, t]);
  };
  // Common prefix/suffix first: the LCS table only covers the changed middle.
  let p = 0;
  while (p < A.length && p < B.length && A[p] === B[p]) p++;
  let s = 0;
  while (s < A.length - p && s < B.length - p && A[A.length - 1 - s] === B[B.length - 1 - s]) s++;
  const x = A.slice(p, A.length - s);
  const y = B.slice(p, B.length - s);
  const n = x.length, m = y.length, w = m + 1;
  A.slice(0, p).forEach((t) => push("=", t));
  if (n * m > 4e6) {
    // ponytail: quadratic LCS; huge rewrites show as one delete + one insert.
    x.forEach((t) => push("-", t));
    y.forEach((t) => push("+", t));
  } else {
    const L = new Uint32Array((n + 1) * w); // L[i*w+j] = LCS length of x[i..], y[j..]
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        L[i * w + j] = x[i] === y[j] ? L[(i + 1) * w + j + 1] + 1 : Math.max(L[(i + 1) * w + j], L[i * w + j + 1]);
    let i = 0, j = 0;
    while (i < n && j < m) {
      if (x[i] === y[j]) { push("=", x[i++]); j++; }
      else if (L[(i + 1) * w + j] >= L[i * w + j + 1]) push("-", x[i++]);
      else push("+", y[j++]);
    }
    while (i < n) push("-", x[i++]);
    while (j < m) push("+", y[j++]);
  }
  A.slice(A.length - s).forEach((t) => push("=", t));
  return out;
}

if (typeof module !== "undefined" && require.main === module) {
  const assert = require("assert");
  const d = wordDiff("this are a test wich I wrote", "This is a test which I wrote");
  assert.deepStrictEqual(d, [
    ["-", "this"], ["+", "This"], ["=", " "], ["-", "are"], ["+", "is"], ["=", " a test "], ["-", "wich"], ["+", "which"], ["=", " I wrote"],
  ]);
  // Old text = "=" + "-" runs, new text = "=" + "+" runs.
  const join = (ops, keep) => ops.filter(([o]) => o === "=" || o === keep).map(([, t]) => t).join("");
  for (const [a, b] of [["", "new"], ["old", ""], ["same  text\n", "same  text\n"], ["a b c", "c b a"]]) {
    const ops = wordDiff(a, b);
    assert.strictEqual(join(ops, "-"), a);
    assert.strictEqual(join(ops, "+"), b);
  }
  console.log("diff.js ok");
}
