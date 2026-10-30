//! Users & roles: list them, see what they can do, and create / grant /
//! revoke / drop with previewed SQL (Postgres family, MySQL family, SQL Server).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{backend::Expect, config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleInfo {
    /// MySQL: `user@host`; elsewhere the role name
    pub id: String,
    pub name: String,
    pub host: Option<String>,
    pub login: bool,
    pub superuser: bool,
    /// CREATEDB, REPLICATION, valid until …, locked …
    pub attributes: Vec<String>,
    pub member_of: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub schema: String,
    /// table / view name; empty for schema- or database-wide grants
    pub object: String,
    pub privileges: Vec<String>,
    /// the engine's own statement, when it reports grants that way (MySQL)
    pub statement: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
pub enum RoleOp {
    Create {
        name: String,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default = "yes")]
        login: bool,
        #[serde(default)]
        superuser: bool,
        #[serde(default)]
        create_db: bool,
    },
    Drop {
        id: String,
    },
    Password {
        id: String,
        password: String,
    },
    Grant {
        id: String,
        privileges: Vec<String>,
        schema: String,
        /// None = every table in the schema / database
        #[serde(default)]
        table: Option<String>,
    },
    Revoke {
        id: String,
        privileges: Vec<String>,
        schema: String,
        #[serde(default)]
        table: Option<String>,
    },
    AddMember {
        id: String,
        role: String,
    },
}

fn yes() -> bool {
    true
}

const PRIVS: &[&str] = &[
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
    "ALL",
    "ALL PRIVILEGES",
    "EXECUTE",
    "CREATE",
    "ALTER",
    "DROP",
    "INDEX",
    "USAGE",
];

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}
fn t(v: Option<&Value>) -> bool {
    matches!(
        s(v).to_lowercase().as_str(),
        "1" | "true" | "t" | "y" | "yes"
    )
}

impl Connection {
    fn roles_supported(&self) -> Result<()> {
        match self.driver() {
            Driver::Postgres | Driver::Mysql | Driver::Mssql => Ok(()),
            _ => Err(Error::Other(
                "users & roles are available for PostgreSQL, MySQL/MariaDB and SQL Server".into(),
            )),
        }
    }

    async fn rows_of(&self, sql: &str) -> Result<Vec<Vec<Value>>> {
        Ok(self
            .query_admin(sql)
            .await?
            .results
            .into_iter()
            .next()
            .map(|r| r.rows)
            .unwrap_or_default())
    }

    /// `user@host` → quoted account for MySQL.
    fn account(&self, id: &str) -> String {
        match self.driver() {
            Driver::Mysql => {
                let (u, h) = id.rsplit_once('@').unwrap_or((id, "%"));
                format!("{}@{}", self.literal(u), self.literal(h))
            }
            _ => self.ident(id),
        }
    }

    pub async fn list_roles(&self) -> Result<Vec<RoleInfo>> {
        self.roles_supported()?;
        match self.driver() {
            Driver::Postgres => {
                let rows = self
                    .rows_of(
                        "SELECT r.rolname, r.rolcanlogin, r.rolsuper, r.rolcreatedb, r.rolcreaterole, r.rolreplication, \
                           r.rolconnlimit, r.rolvaliduntil::text, \
                           ARRAY(SELECT b.rolname FROM pg_auth_members m JOIN pg_roles b ON b.oid = m.roleid \
                                 WHERE m.member = r.oid ORDER BY 1)::text \
                         FROM pg_roles r WHERE r.rolname !~ '^pg_' ORDER BY r.rolcanlogin DESC, r.rolname",
                    )
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        let mut attributes = vec![];
                        for (i, a) in [(3, "CREATEDB"), (4, "CREATEROLE"), (5, "REPLICATION")] {
                            if t(r.get(i)) {
                                attributes.push(a.to_string());
                            }
                        }
                        let limit = s(r.get(6));
                        if !limit.is_empty() && limit != "-1" {
                            attributes.push(format!("connection limit {limit}"));
                        }
                        let until = s(r.get(7));
                        if !until.is_empty() && until != "infinity" {
                            attributes.push(format!("valid until {until}"));
                        }
                        let members = s(r.get(8));
                        RoleInfo {
                            id: s(r.first()),
                            name: s(r.first()),
                            host: None,
                            login: t(r.get(1)),
                            superuser: t(r.get(2)),
                            attributes,
                            member_of: members
                                .trim_matches(|c| c == '{' || c == '}')
                                .split(',')
                                .filter(|x| !x.is_empty())
                                .map(|x| x.trim_matches('"').to_string())
                                .collect(),
                        }
                    })
                    .collect())
            }
            Driver::Mysql => {
                let rows = match self
                    .rows_of(
                        "SELECT User, Host, Super_priv, Create_priv, Grant_priv, \
                           COALESCE(account_locked, 'N') FROM mysql.user ORDER BY User, Host",
                    )
                    .await
                {
                    Ok(r) => r,
                    // MariaDB < 10.4 has no account_locked; restricted users can't read mysql.user
                    Err(_) => match self
                        .rows_of("SELECT User, Host, Super_priv, Create_priv, Grant_priv, 'N' FROM mysql.user ORDER BY User, Host")
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => {
                            return Err(Error::Other(
                                "this account can't read mysql.user — connect as an admin to manage users".into(),
                            ))
                        }
                    },
                };
                Ok(rows
                    .into_iter()
                    .filter(|r| !s(r.first()).starts_with("mysql.") && !s(r.first()).is_empty())
                    .map(|r| {
                        let (u, h) = (s(r.first()), s(r.get(1)));
                        let mut attributes = vec![];
                        if t(r.get(3)) {
                            attributes.push("CREATE".into());
                        }
                        if t(r.get(4)) {
                            attributes.push("GRANT OPTION".into());
                        }
                        if t(r.get(5)) {
                            attributes.push("locked".into());
                        }
                        RoleInfo {
                            id: format!("{u}@{h}"),
                            name: u,
                            host: Some(h),
                            login: !t(r.get(5)),
                            superuser: t(r.get(2)),
                            attributes,
                            member_of: vec![],
                        }
                    })
                    .collect())
            }
            _ => {
                let rows = self
                    .rows_of(
                        "SELECT p.name, p.type_desc, \
                           CASE WHEN p.type IN ('S','U','E','G') THEN 1 ELSE 0 END, \
                           IS_ROLEMEMBER('db_owner', p.name), \
                           STUFF((SELECT ',' + r.name FROM sys.database_role_members m \
                             JOIN sys.database_principals r ON r.principal_id = m.role_principal_id \
                             WHERE m.member_principal_id = p.principal_id FOR XML PATH('')), 1, 1, '') \
                         FROM sys.database_principals p \
                         WHERE p.type IN ('S','U','E','G','R','X') AND p.is_fixed_role = 0 \
                           AND p.name NOT IN ('guest','INFORMATION_SCHEMA','sys','public') \
                         ORDER BY 3 DESC, 1",
                    )
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|r| RoleInfo {
                        id: s(r.first()),
                        name: s(r.first()),
                        host: None,
                        login: t(r.get(2)),
                        superuser: t(r.get(3)),
                        attributes: vec![s(r.get(1)).replace('_', " ").to_lowercase()],
                        member_of: s(r.get(4))
                            .split(',')
                            .filter(|x| !x.is_empty())
                            .map(String::from)
                            .collect(),
                    })
                    .collect())
            }
        }
    }

    pub async fn role_grants(&self, id: &str) -> Result<Vec<Grant>> {
        self.roles_supported()?;
        let lit = self.literal(id);
        match self.driver() {
            Driver::Postgres => {
                let rows = self
                    .rows_of(&format!(
                        "SELECT table_schema, table_name, string_agg(privilege_type, ',' ORDER BY privilege_type) \
                         FROM information_schema.table_privileges WHERE grantee = {lit} \
                         GROUP BY 1, 2 ORDER BY 1, 2"
                    ))
                    .await?;
                let mut out: Vec<Grant> = rows
                    .into_iter()
                    .map(|r| Grant {
                        schema: s(r.first()),
                        object: s(r.get(1)),
                        privileges: s(r.get(2)).split(',').map(String::from).collect(),
                        statement: None,
                    })
                    .collect();
                // schema-level USAGE / CREATE
                for r in self
                    .rows_of(&format!(
                        "SELECT n.nspname, has_schema_privilege({lit}, n.oid, 'USAGE'), has_schema_privilege({lit}, n.oid, 'CREATE') \
                         FROM pg_namespace n WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' ORDER BY 1"
                    ))
                    .await
                    .unwrap_or_default()
                {
                    let mut p = vec![];
                    if t(r.get(1)) {
                        p.push("USAGE".to_string());
                    }
                    if t(r.get(2)) {
                        p.push("CREATE".to_string());
                    }
                    if !p.is_empty() {
                        out.insert(0, Grant { schema: s(r.first()), object: String::new(), privileges: p, statement: None });
                    }
                }
                Ok(out)
            }
            Driver::Mysql => {
                let rows = self
                    .rows_of(&format!("SHOW GRANTS FOR {}", self.account(id)))
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        let st = s(r.first());
                        // GRANT SELECT, INSERT ON `db`.`t` TO …
                        let (privs, on) = st
                            .strip_prefix("GRANT ")
                            .and_then(|x| x.split_once(" ON "))
                            .map(|(p, rest)| {
                                (
                                    p.to_string(),
                                    rest.split(" TO ").next().unwrap_or("").to_string(),
                                )
                            })
                            .unwrap_or_default();
                        let (schema, object) = on.split_once('.').unwrap_or((&on, ""));
                        Grant {
                            schema: schema.trim_matches('`').to_string(),
                            object: object.trim_matches('`').replace('*', ""),
                            privileges: privs.split(", ").map(String::from).collect(),
                            statement: Some(st),
                        }
                    })
                    .collect())
            }
            _ => {
                let rows = self
                    .rows_of(&format!(
                        "SELECT COALESCE(OBJECT_SCHEMA_NAME(p.major_id), s.name, ''), COALESCE(OBJECT_NAME(p.major_id), ''), \
                           p.permission_name, p.state_desc, p.class_desc \
                         FROM sys.database_permissions p JOIN sys.database_principals u ON u.principal_id = p.grantee_principal_id \
                         LEFT JOIN sys.schemas s ON p.class = 3 AND s.schema_id = p.major_id \
                         WHERE u.name = {lit} ORDER BY 1, 2, 3"
                    ))
                    .await?;
                let mut out: Vec<Grant> = vec![];
                for r in rows {
                    let (sc, ob) = (
                        s(r.first()),
                        if s(r.get(4)) == "SCHEMA" {
                            String::new()
                        } else {
                            s(r.get(1))
                        },
                    );
                    let p = if s(r.get(3)).starts_with("DENY") {
                        format!("DENY {}", s(r.get(2)))
                    } else {
                        s(r.get(2))
                    };
                    match out.iter_mut().find(|g| g.schema == sc && g.object == ob) {
                        Some(g) => g.privileges.push(p),
                        None => out.push(Grant {
                            schema: sc,
                            object: ob,
                            privileges: vec![p],
                            statement: None,
                        }),
                    }
                }
                Ok(out)
            }
        }
    }

    fn check_privs(privs: &[String]) -> Result<Vec<String>> {
        if privs.is_empty() {
            return Err(Error::Other("pick at least one privilege".into()));
        }
        privs
            .iter()
            .map(|p| {
                let u = p.trim().to_uppercase();
                if PRIVS.contains(&u.as_str()) {
                    Ok(u)
                } else {
                    Err(Error::Other(format!("unknown privilege `{p}`")))
                }
            })
            .collect()
    }

    /// SQL for a role operation. With `mask`, passwords are shown as `'••••••'`.
    pub fn role_sql(&self, op: &RoleOp, mask: bool) -> Result<Vec<String>> {
        self.roles_supported()?;
        let d = self.driver();
        let pw = |p: &str| {
            if mask {
                "'••••••'".to_string()
            } else {
                self.literal(p)
            }
        };
        let on = |schema: &str, table: &Option<String>| -> String {
            match (d, table.as_deref().filter(|x| !x.is_empty())) {
                (Driver::Postgres, Some(t)) => {
                    format!("TABLE {}.{}", self.ident(schema), self.ident(t))
                }
                (Driver::Postgres, None) => format!("ALL TABLES IN SCHEMA {}", self.ident(schema)),
                (Driver::Mysql, Some(t)) => format!("{}.{}", self.ident(schema), self.ident(t)),
                (Driver::Mysql, None) => format!("{}.*", self.ident(schema)),
                (_, Some(t)) => format!("{}.{}", self.ident(schema), self.ident(t)),
                (_, None) => format!("SCHEMA::{}", self.ident(schema)),
            }
        };
        Ok(match op {
            RoleOp::Create {
                name,
                host,
                password,
                login,
                superuser,
                create_db,
            } => {
                if name.trim().is_empty() {
                    return Err(Error::Other("enter a name".into()));
                }
                let name = name.trim();
                match d {
                    Driver::Postgres => {
                        let mut opts = vec![if *login { "LOGIN" } else { "NOLOGIN" }.to_string()];
                        if *superuser {
                            opts.push("SUPERUSER".into());
                        }
                        if *create_db {
                            opts.push("CREATEDB".into());
                        }
                        if let Some(p) = password.as_deref().filter(|p| !p.is_empty()) {
                            opts.push(format!("PASSWORD {}", pw(p)));
                        }
                        vec![format!(
                            "CREATE ROLE {} WITH {}",
                            self.ident(name),
                            opts.join(" ")
                        )]
                    }
                    Driver::Mysql => {
                        let acct = self.account(&format!(
                            "{name}@{}",
                            host.as_deref().filter(|h| !h.is_empty()).unwrap_or("%")
                        ));
                        let mut v = vec![match password.as_deref().filter(|p| !p.is_empty()) {
                            Some(p) => format!("CREATE USER {acct} IDENTIFIED BY {}", pw(p)),
                            None => format!("CREATE USER {acct}"),
                        }];
                        if *superuser {
                            v.push(format!(
                                "GRANT ALL PRIVILEGES ON *.* TO {acct} WITH GRANT OPTION"
                            ));
                        } else if *create_db {
                            v.push(format!("GRANT CREATE ON *.* TO {acct}"));
                        }
                        v
                    }
                    _ => {
                        let n = self.ident(name);
                        if *login {
                            let p =
                                password
                                    .as_deref()
                                    .filter(|p| !p.is_empty())
                                    .ok_or_else(|| {
                                        Error::Other("SQL Server logins need a password".into())
                                    })?;
                            let mut v = vec![
                                format!("CREATE LOGIN {n} WITH PASSWORD = {}", pw(p)),
                                format!("CREATE USER {n} FOR LOGIN {n}"),
                            ];
                            if *superuser {
                                v.push(format!("ALTER ROLE db_owner ADD MEMBER {n}"));
                            }
                            v
                        } else {
                            vec![format!("CREATE ROLE {n}")]
                        }
                    }
                }
            }
            RoleOp::Drop { id } => match d {
                Driver::Postgres => vec![format!("DROP ROLE {}", self.ident(id))],
                Driver::Mysql => vec![format!("DROP USER {}", self.account(id))],
                _ => vec![format!("DROP USER {}", self.ident(id))],
            },
            RoleOp::Password { id, password } => {
                if password.is_empty() {
                    return Err(Error::Other("enter the new password".into()));
                }
                match d {
                    Driver::Postgres => vec![format!(
                        "ALTER ROLE {} WITH PASSWORD {}",
                        self.ident(id),
                        pw(password)
                    )],
                    Driver::Mysql => vec![format!(
                        "ALTER USER {} IDENTIFIED BY {}",
                        self.account(id),
                        pw(password)
                    )],
                    _ => vec![format!(
                        "ALTER LOGIN {} WITH PASSWORD = {}",
                        self.ident(id),
                        pw(password)
                    )],
                }
            }
            RoleOp::Grant {
                id,
                privileges,
                schema,
                table,
            } => {
                let p = Self::check_privs(privileges)?.join(", ");
                let mut v = vec![];
                if d == Driver::Postgres {
                    v.push(format!(
                        "GRANT USAGE ON SCHEMA {} TO {}",
                        self.ident(schema),
                        self.ident(id)
                    ));
                }
                v.push(format!(
                    "GRANT {p} ON {} TO {}",
                    on(schema, table),
                    self.account(id)
                ));
                v
            }
            RoleOp::Revoke {
                id,
                privileges,
                schema,
                table,
            } => {
                let p = Self::check_privs(privileges)?.join(", ");
                vec![format!(
                    "REVOKE {p} ON {} FROM {}",
                    on(schema, table),
                    self.account(id)
                )]
            }
            RoleOp::AddMember { id, role } => match d {
                Driver::Postgres => {
                    vec![format!("GRANT {} TO {}", self.ident(role), self.ident(id))]
                }
                Driver::Mysql => vec![format!(
                    "GRANT {} TO {}",
                    self.account(role),
                    self.account(id)
                )],
                _ => vec![format!(
                    "ALTER ROLE {} ADD MEMBER {}",
                    self.ident(role),
                    self.ident(id)
                )],
            },
        })
    }

    pub async fn apply_role(&self, op: &RoleOp) -> Result<Vec<String>> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let stmts = self.role_sql(op, false)?;
        // CREATE LOGIN / DROP USER can't run inside a transaction on some engines
        for st in &stmts {
            self.exec_tx(vec![(Expect::Any, st.clone())]).await?;
        }
        self.role_sql(op, true)
    }
}
