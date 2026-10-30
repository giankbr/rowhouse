// Table stats — a sample Rowhouse extension.
// Uses window.rowhouse (see docs/EXTENSIONS.md): tables(), query(), openTable().
const $ = (s) => document.querySelector(s);

function quote(dialect, name) {
  if (dialect === "mysql" || dialect === "clickhouse") return "`" + name.replace(/`/g, "``") + "`";
  if (dialect === "mssql") return "[" + name.replace(/]/g, "]]") + "]";
  return '"' + name.replace(/"/g, '""') + '"';
}

async function load() {
  const ctx = await rowhouse.context();
  $("#conn").textContent = ctx.connection ? `· ${ctx.connection.name}` : "· no connection";
  if (!ctx.connection) return;
  const dialect = ctx.connection.dialect;
  const tables = (await rowhouse.tables()).filter((t) => t.kind === "table").slice(0, 200);
  $("#status").textContent = `Counting rows in ${tables.length} tables…`;
  const rows = [];
  for (const t of tables) {
    const target = dialect === "sqlite" ? quote(dialect, t.name) : `${quote(dialect, t.schema)}.${quote(dialect, t.name)}`;
    try {
      const out = await rowhouse.query(`SELECT COUNT(*) FROM ${target}`);
      rows.push({ ...t, count: Number(out.results[0].rows[0][0]) });
    } catch (e) {
      rows.push({ ...t, count: null, error: String(e.message || e) });
    }
  }
  rows.sort((a, b) => (b.count ?? -1) - (a.count ?? -1));
  const max = Math.max(1, ...rows.map((r) => r.count ?? 0));
  const body = $("#out tbody");
  body.innerHTML = "";
  for (const r of rows) {
    const tr = document.createElement("tr");
    tr.className = "row";
    tr.title = r.error || "Open table";
    tr.innerHTML = `<td><span class="schema"></span><span class="name"></span></td><td class="num"></td><td class="bar"><span></span></td>`;
    tr.querySelector(".schema").textContent = r.schema && r.schema !== "main" ? r.schema + "." : "";
    tr.querySelector(".name").textContent = r.name;
    tr.querySelector(".num").textContent = r.count === null ? "—" : r.count.toLocaleString();
    tr.querySelector(".bar span").style.width = `${((r.count ?? 0) / max) * 100}%`;
    tr.onclick = () => rowhouse.openTable(r.schema, r.name);
    body.appendChild(tr);
  }
  $("#out").hidden = false;
  $("#status").textContent = `${rows.reduce((s, r) => s + (r.count ?? 0), 0).toLocaleString()} rows in ${rows.length} tables`;
}

$("#refresh").onclick = () => load().catch((e) => rowhouse.toast(String(e.message || e), "err"));
rowhouse.onConnection(() => load());
load().catch((e) => ($("#status").textContent = String(e.message || e)));
