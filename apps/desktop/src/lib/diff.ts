/** Minimal line diff (LCS) for showing AI proposals against the editor SQL. */
export type DiffLine = { kind: "same" | "add" | "del"; text: string };

export function lineDiff(a: string, b: string): DiffLine[] {
  const x = a.replace(/\s+$/, "").split("\n");
  const y = b.replace(/\s+$/, "").split("\n");
  const n = x.length, m = y.length;
  // dp[i][j] = LCS length of x[i..], y[j..]
  const dp = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i].trim() === y[j].trim() ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: DiffLine[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (x[i].trim() === y[j].trim()) (out.push({ kind: "same", text: y[j] }), i++, j++);
    else if (dp[i + 1][j] >= dp[i][j + 1]) out.push({ kind: "del", text: x[i++] });
    else out.push({ kind: "add", text: y[j++] });
  }
  while (i < n) out.push({ kind: "del", text: x[i++] });
  while (j < m) out.push({ kind: "add", text: y[j++] });
  return out;
}
