/** SQL pretty-printing in the connection's dialect (sql-formatter). */
import type { Dialect } from "./api";

const LANG: Partial<Record<Dialect, string>> = {
  postgres: "postgresql",
  mysql: "mysql",
  sqlite: "sqlite",
  duckdb: "duckdb",
  mssql: "transactsql",
  clickhouse: "clickhouse",
  cql: "sql",
  oracle: "plsql",
};

export function canFormat(d: Dialect) {
  return d in LANG;
}

export async function formatSql(sql: string, d: Dialect): Promise<string> {
  const { format } = await import("sql-formatter");
  const language = LANG[d] ?? "sql";
  try {
    return format(sql, { language: language as never, keywordCase: "upper", tabWidth: 2, linesBetweenQueries: 1 });
  } catch {
    // unknown dialect bits: fall back to generic SQL
    return format(sql, { language: "sql", keywordCase: "upper", tabWidth: 2 });
  }
}
