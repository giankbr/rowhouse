//! MongoDB (beta). Collections show up as tables; the editor takes mongosh-style
//! statements (see [`super::mongo_shell`]); documents are flattened to their
//! top-level fields for the grid, nested values shown as JSON.

use std::time::Duration;

use futures_util::TryStreamExt;
use mongodb::{
    bson::{self, doc, oid::ObjectId, Bson, Document},
    options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions},
    Client as MClient, Database, IndexModel,
};
use serde_json::{Map, Number, Value};

use super::mongo_shell::{self as shell, Stmt};
use crate::{
    browse::{Filter, FilterOp, PageRequest},
    changes::ChangeSet,
    config::{TlsConfig, TlsMode},
    ColumnInfo, Error, QueryResult, Result, ResultColumn, TableInfo,
};

#[derive(Clone)]
pub struct Client {
    client: MClient,
    db: String,
}

fn merr(e: mongodb::error::Error) -> Error {
    Error::Other(format!("mongodb: {e}"))
}

/// Methods allowed on read-only connections and for MCP.
const READ_METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "countDocuments",
    "count",
    "estimatedDocumentCount",
    "distinct",
    "getIndexes",
    "stats",
];

/// Commands allowed through `db.runCommand` on read-only connections.
const READ_COMMANDS: &[&str] = &[
    "ping",
    "buildinfo",
    "serverstatus",
    "dbstats",
    "collstats",
    "listcollections",
    "listindexes",
    "listdatabases",
    "hostinfo",
    "connectionstatus",
    "count",
    "distinct",
    "find",
    "explain",
];

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        db: Option<&str>,
        tls: Option<&TlsConfig>,
    ) -> Result<Self> {
        let user = user.filter(|u| !u.is_empty());
        let mut o = if host.starts_with("mongodb://") || host.starts_with("mongodb+srv://") {
            // a full connection string in the host field (Atlas etc.)
            ClientOptions::parse(host).await.map_err(merr)?
        } else {
            let mut o = ClientOptions::default();
            o.hosts = vec![ServerAddress::Tcp {
                host: host.to_string(),
                port: Some(port),
            }];
            // talk to exactly this server (also what you want through an SSH tunnel)
            o.direct_connection = Some(true);
            o
        };
        if o.credential.is_none() {
            if let Some(u) = user {
                o.credential = Some(
                    Credential::builder()
                        .username(u.to_string())
                        .password(password.map(str::to_string))
                        .source("admin".to_string())
                        .build(),
                );
            }
        } else if let (Some(c), Some(p)) = (o.credential.as_mut(), password) {
            if c.password.is_none() && !p.is_empty() {
                c.password = Some(p.to_string());
            }
        }
        if let Some(t) = tls {
            if t.mode != TlsMode::Disable && o.tls.is_none() {
                let path = |p: &Option<String>| {
                    p.as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .map(std::path::PathBuf::from)
                };
                let mut to = TlsOptions::default();
                to.ca_file_path = path(&t.ca_cert);
                // MongoDB wants certificate + key in one PEM; use the cert file if it has both
                to.cert_key_file_path = path(&t.client_cert);
                to.allow_invalid_certificates =
                    Some(matches!(t.mode, TlsMode::Prefer | TlsMode::Require));
                if t.mode != TlsMode::Prefer {
                    o.tls = Some(Tls::Enabled(to));
                }
            }
        }
        o.app_name = Some("Rowhouse".into());
        o.connect_timeout = Some(Duration::from_secs(10));
        o.server_selection_timeout = Some(Duration::from_secs(10));
        let default_db = o.default_database.clone();
        let client = MClient::with_options(o).map_err(merr)?;
        let db = db
            .filter(|d| !d.trim().is_empty())
            .map(str::to_string)
            .or(default_db)
            .unwrap_or_else(|| "test".into());
        let c = Self { client, db };
        c.database()
            .run_command(doc! { "ping": 1 })
            .await
            .map_err(merr)?;
        Ok(c)
    }

    fn database(&self) -> Database {
        self.client.database(&self.db)
    }

    pub fn db_name(&self) -> &str {
        &self.db
    }

    pub async fn ping(&self) -> Result<()> {
        self.database()
            .run_command(doc! { "ping": 1 })
            .await
            .map(|_| ())
            .map_err(merr)
    }

    pub async fn collections(&self) -> Result<Vec<TableInfo>> {
        let mut cur = self.database().list_collections().await.map_err(merr)?;
        let mut out = Vec::new();
        while let Some(spec) = cur.try_next().await.map_err(merr)? {
            if spec.name.starts_with("system.") {
                continue;
            }
            let view = matches!(spec.collection_type, mongodb::results::CollectionType::View);
            out.push(TableInfo {
                schema: self.db.clone(),
                name: spec.name,
                kind: if view { "view" } else { "table" }.into(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Fields seen in a sample of documents, `_id` first.
    pub async fn describe(&self, coll: &str) -> Result<Vec<ColumnInfo>> {
        let mut cur = self
            .database()
            .collection::<Document>(coll)
            .find(doc! {})
            .limit(200)
            .await
            .map_err(merr)?;
        let mut docs = Vec::new();
        while let Some(d) = cur.try_next().await.map_err(merr)? {
            docs.push(d);
        }
        let n = docs.len();
        let mut cols: Vec<(String, String, usize)> = vec![("_id".into(), String::new(), 0)];
        for d in &docs {
            for (k, v) in d {
                let pos = match cols.iter().position(|c| &c.0 == k) {
                    Some(p) => p,
                    None => {
                        cols.push((k.clone(), String::new(), 0));
                        cols.len() - 1
                    }
                };
                let c = &mut cols[pos];
                if !matches!(v, Bson::Null) {
                    c.2 += 1;
                    let t = type_name(v);
                    if c.1.is_empty() {
                        c.1 = t.into();
                    } else if c.1 != t && !c.1.contains(t) {
                        c.1 = format!("{} | {t}", c.1);
                    }
                }
            }
        }
        Ok(cols
            .into_iter()
            .map(|(name, ty, seen)| ColumnInfo {
                primary_key: name == "_id",
                nullable: name != "_id" && seen < n,
                data_type: if ty.is_empty() { "objectId".into() } else { ty },
                default: None,
                name,
            })
            .collect())
    }

    pub async fn run(
        &self,
        script: &str,
        limit: usize,
        read_only: bool,
    ) -> Result<Vec<QueryResult>> {
        let stmts = shell::parse(script).map_err(Error::Other)?;
        if stmts.is_empty() {
            return Err(Error::Other("nothing to run".into()));
        }
        if read_only {
            for s in &stmts {
                check_read_only(s).map_err(Error::ReadOnly)?;
            }
        }
        let mut out = Vec::new();
        for s in stmts {
            out.push(self.exec(s, limit).await?);
        }
        Ok(out)
    }

    async fn exec(&self, s: Stmt, limit: usize) -> Result<QueryResult> {
        let db = self.database();
        match s {
            Stmt::ShowCollections | Stmt::CollectionNames => {
                let names = db.list_collection_names().await.map_err(merr)?;
                Ok(single_column("name", names.into_iter().map(Value::String)))
            }
            Stmt::ShowDatabases => {
                let names = self.client.list_database_names().await.map_err(merr)?;
                Ok(single_column("name", names.into_iter().map(Value::String)))
            }
            Stmt::Command { admin, doc } => {
                let target = if admin {
                    self.client.database("admin")
                } else {
                    db
                };
                let r = target.run_command(doc).await.map_err(merr)?;
                Ok(docs_result(vec![r], limit))
            }
            Stmt::Call {
                coll,
                method,
                args,
                chain,
            } => self.call(&coll, &method, args, chain, limit).await,
        }
    }

    async fn call(
        &self,
        coll: &str,
        method: &str,
        args: Vec<Bson>,
        chain: Vec<(String, Vec<Bson>)>,
        limit: usize,
    ) -> Result<QueryResult> {
        let c = self.database().collection::<Document>(coll);
        let mut a = args.into_iter();
        let mut doc_arg = |what: &str| -> Result<Document> {
            match a.next() {
                None => Ok(Document::new()),
                Some(Bson::Document(d)) => Ok(d),
                Some(other) => Err(Error::Other(format!(
                    "{method}: {what} must be a document, got {}",
                    shell::shell(&other)
                ))),
            }
        };
        let affected = |n: u64, extra: Document| {
            let mut d = doc! { "acknowledged": true };
            d.extend(extra);
            let mut r = docs_result(vec![d], 1);
            r.rows_affected = n;
            r
        };
        match method {
            "find" | "findOne" => {
                let filter = doc_arg("filter")?;
                let projection = doc_arg("projection")?;
                let mut sort = None;
                let mut skip = None;
                let mut lim = if method == "findOne" { Some(1) } else { None };
                let mut projection = (!projection.is_empty()).then_some(projection);
                let mut count = false;
                for (m, args) in chain {
                    let first = args.into_iter().next();
                    match (m.as_str(), first) {
                        ("sort", Some(Bson::Document(d))) => sort = Some(d),
                        ("limit", Some(n)) => lim = Some(as_i64(&n)?),
                        ("skip", Some(n)) => skip = Some(as_i64(&n)? as u64),
                        ("projection" | "project", Some(Bson::Document(d))) => projection = Some(d),
                        ("count" | "itcount" | "size", _) => count = true,
                        ("toArray" | "pretty" | "batchSize" | "hint" | "maxTimeMS" | "comment", _) => {}
                        (other, _) => {
                            return Err(Error::Other(format!("unsupported cursor method `.{other}()`")))
                        }
                    }
                }
                if count {
                    let n = c.count_documents(filter).await.map_err(merr)?;
                    return Ok(single_column("count", [Value::from(n)]));
                }
                let cap = lim.map(|l| l.unsigned_abs() as usize).unwrap_or(limit).min(limit.max(1));
                let mut f = c.find(filter).limit(cap as i64 + 1);
                if let Some(s) = sort {
                    f = f.sort(s);
                }
                if let Some(s) = skip {
                    f = f.skip(s);
                }
                if let Some(p) = projection {
                    f = f.projection(p);
                }
                let mut cur = f.await.map_err(merr)?;
                let mut docs = Vec::new();
                while let Some(d) = cur.try_next().await.map_err(merr)? {
                    docs.push(d);
                }
                let mut r = docs_result(docs, cap);
                // the extra document only tells us there were more than the limit
                if lim.is_some_and(|l| l.unsigned_abs() as usize <= limit) {
                    r.truncated = false;
                }
                Ok(r)
            }
            "aggregate" => {
                let pipeline: Vec<Document> = match a.next() {
                    Some(Bson::Array(stages)) => stages
                        .into_iter()
                        .map(|s| match s {
                            Bson::Document(d) => Ok(d),
                            _ => Err(Error::Other("aggregate: each stage must be a document".into())),
                        })
                        .collect::<Result<_>>()?,
                    None => Vec::new(),
                    _ => return Err(Error::Other("aggregate needs an array of stages".into())),
                };
                let mut cur = c.aggregate(pipeline).await.map_err(merr)?;
                let mut docs = Vec::new();
                while let Some(d) = cur.try_next().await.map_err(merr)? {
                    docs.push(d);
                    if docs.len() > limit {
                        break;
                    }
                }
                Ok(docs_result(docs, limit))
            }
            "countDocuments" | "count" => {
                let n = c.count_documents(doc_arg("filter")?).await.map_err(merr)?;
                Ok(single_column("count", [Value::from(n)]))
            }
            "estimatedDocumentCount" => {
                let n = c.estimated_document_count().await.map_err(merr)?;
                Ok(single_column("count", [Value::from(n)]))
            }
            "distinct" => {
                let field = match a.next() {
                    Some(Bson::String(s)) => s,
                    _ => return Err(Error::Other("distinct needs a field name".into())),
                };
                let filter = match a.next() {
                    Some(Bson::Document(d)) => d,
                    _ => Document::new(),
                };
                let vals = c.distinct(&field, filter).await.map_err(merr)?;
                Ok(single_column(&field, vals.iter().map(to_json)))
            }
            "getIndexes" => {
                let mut cur = c.list_indexes().await.map_err(merr)?;
                let mut docs = Vec::new();
                while let Some(ix) = cur.try_next().await.map_err(merr)? {
                    let mut d = doc! { "key": ix.keys.clone() };
                    if let Some(o) = &ix.options {
                        if let Ok(od) = bson::to_document(o) {
                            d.extend(od);
                        }
                    }
                    docs.push(d);
                }
                Ok(docs_result(docs, limit))
            }
            "stats" => {
                let r = self
                    .database()
                    .run_command(doc! { "collStats": coll })
                    .await
                    .map_err(merr)?;
                Ok(docs_result(vec![r], 1))
            }
            "insertOne" => {
                let d = doc_arg("document")?;
                let r = c.insert_one(d).await.map_err(merr)?;
                Ok(affected(1, doc! { "insertedId": r.inserted_id }))
            }
            "insertMany" => {
                let docs: Vec<Document> = match a.next() {
                    Some(Bson::Array(items)) => items
                        .into_iter()
                        .filter_map(|b| match b {
                            Bson::Document(d) => Some(d),
                            _ => None,
                        })
                        .collect(),
                    _ => return Err(Error::Other("insertMany needs an array of documents".into())),
                };
                if docs.is_empty() {
                    return Err(Error::Other("insertMany: no documents".into()));
                }
                let r = c.insert_many(docs).await.map_err(merr)?;
                Ok(affected(r.inserted_ids.len() as u64, Document::new()))
            }
            "updateOne" | "updateMany" => {
                let filter = doc_arg("filter")?;
                let update = a.next().ok_or_else(|| Error::Other(format!("{method} needs an update")))?;
                let upsert = matches!(a.next(), Some(Bson::Document(o)) if o.get_bool("upsert").unwrap_or(false));
                let update: mongodb::options::UpdateModifications = match update {
                    Bson::Document(d) => d.into(),
                    Bson::Array(stages) => stages
                        .into_iter()
                        .filter_map(|s| match s {
                            Bson::Document(d) => Some(d),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .into(),
                    _ => return Err(Error::Other("update must be a document or pipeline".into())),
                };
                let r = if method == "updateOne" {
                    c.update_one(filter, update).upsert(upsert).await
                } else {
                    c.update_many(filter, update).upsert(upsert).await
                }
                .map_err(merr)?;
                let mut extra = doc! {
                    "matchedCount": r.matched_count as i64,
                    "modifiedCount": r.modified_count as i64,
                };
                if let Some(id) = r.upserted_id {
                    extra.insert("upsertedId", id);
                }
                Ok(affected(r.modified_count, extra))
            }
            "replaceOne" => {
                let filter = doc_arg("filter")?;
                let repl = doc_arg("replacement")?;
                let r = c.replace_one(filter, repl).await.map_err(merr)?;
                Ok(affected(
                    r.modified_count,
                    doc! { "matchedCount": r.matched_count as i64, "modifiedCount": r.modified_count as i64 },
                ))
            }
            "deleteOne" | "deleteMany" => {
                let filter = doc_arg("filter")?;
                let r = if method == "deleteOne" {
                    c.delete_one(filter).await
                } else {
                    c.delete_many(filter).await
                }
                .map_err(merr)?;
                Ok(affected(r.deleted_count, doc! { "deletedCount": r.deleted_count as i64 }))
            }
            "drop" => {
                c.drop().await.map_err(merr)?;
                Ok(affected(0, Document::new()))
            }
            "createIndex" => {
                let keys = doc_arg("keys")?;
                let mut m = IndexModel::builder().keys(keys).build();
                if let Some(Bson::Document(o)) = a.next() {
                    m.options = bson::from_document(o).ok();
                }
                let r = c.create_index(m).await.map_err(merr)?;
                Ok(affected(0, doc! { "name": r.index_name }))
            }
            "dropIndex" => {
                match a.next() {
                    Some(Bson::String(name)) => c.drop_index(name).await.map_err(merr)?,
                    _ => return Err(Error::Other("dropIndex needs the index name".into())),
                }
                Ok(affected(0, Document::new()))
            }
            other => Err(Error::Other(format!(
                "unsupported method `{other}()` — supported: find, findOne, aggregate, countDocuments, \
                 distinct, getIndexes, insertOne/Many, updateOne/Many, replaceOne, deleteOne/Many, \
                 createIndex, dropIndex, drop"
            ))),
        }
    }

    /// Browse a collection with grid filters / sort / paging.
    pub async fn page(&self, r: &PageRequest) -> Result<(QueryResult, Option<u64>, String)> {
        let c = self.database().collection::<Document>(&r.table);
        let filter = filter_doc(&r.filters);
        let sort = r
            .sort
            .as_ref()
            .map(|s| doc! { s.column.clone(): if s.desc { -1 } else { 1 } });
        let mut shown = format!(
            "db.{}.find({})",
            coll_ref(&r.table),
            shell::shell(&Bson::Document(filter.clone()))
        );
        let mut f = c.find(filter.clone()).limit(r.limit as i64);
        if let Some(s) = sort {
            shown.push_str(&format!(
                ".sort({})",
                shell::shell(&Bson::Document(s.clone()))
            ));
            f = f.sort(s);
        }
        if r.offset > 0 {
            shown.push_str(&format!(".skip({})", r.offset));
            f = f.skip(r.offset as u64);
        }
        shown.push_str(&format!(".limit({})", r.limit));
        let mut cur = f.await.map_err(merr)?;
        let mut docs = Vec::new();
        while let Some(d) = cur.try_next().await.map_err(merr)? {
            docs.push(d);
        }
        let mut result = docs_result(docs, r.limit);
        result.truncated = false;
        // always show _id first, even when the page is empty
        if result.columns.is_empty() {
            result.columns.push(ResultColumn {
                name: "_id".into(),
                data_type: "objectId".into(),
            });
        }
        let total = if r.count {
            Some(c.count_documents(filter).await.map_err(merr)?)
        } else {
            None
        };
        Ok((result, total, shown))
    }

    /// The mongosh statements a grid change set turns into (preview).
    pub fn preview(&self, cs: &ChangeSet) -> Result<Vec<String>> {
        Ok(self
            .plan(cs)?
            .into_iter()
            .map(|op| op.shell(&cs.table))
            .collect())
    }

    fn plan(&self, cs: &ChangeSet) -> Result<Vec<Op>> {
        let mut ops = Vec::new();
        for u in &cs.updates {
            let id = id_filter(&u.key)?;
            let mut set = Document::new();
            let mut unset = Document::new();
            for (k, v) in &u.set {
                if k == "_id" {
                    return Err(Error::Other("_id can't be changed".into()));
                }
                match v {
                    Value::Null => {
                        unset.insert(k, "");
                    }
                    _ => {
                        set.insert(k, from_json(v));
                    }
                }
            }
            ops.push(Op::Update { id, set, unset });
        }
        for row in &cs.inserts {
            let mut d = Document::new();
            for (k, v) in row {
                if v.is_null() || (k == "_id" && v.as_str().is_some_and(str::is_empty)) {
                    continue; // let the server assign _id
                }
                let b = if k == "_id" {
                    id_value(v)
                } else {
                    from_json(v)
                };
                d.insert(k, b);
            }
            ops.push(Op::Insert(d));
        }
        for key in &cs.deletes {
            ops.push(Op::Delete(id_filter(key)?));
        }
        Ok(ops)
    }

    /// Apply a grid change set. MongoDB only has multi-document transactions on
    /// replica sets, so every targeted document is checked first and the
    /// operations then run in order.
    pub async fn apply(&self, cs: &ChangeSet) -> Result<u64> {
        let c = self.database().collection::<Document>(&cs.table);
        let ops = self.plan(cs)?;
        // pre-flight: every update/delete must hit exactly one document
        for op in &ops {
            if let Op::Update { id, .. } | Op::Delete(id) = op {
                let n = c.count_documents(id.clone()).await.map_err(merr)?;
                if n != 1 {
                    return Err(Error::Other(format!(
                        "expected 1 document for {}, found {n} — nothing was changed",
                        shell::shell(&Bson::Document(id.clone()))
                    )));
                }
            }
        }
        let mut total = 0;
        for op in ops {
            match op {
                Op::Update { id, set, unset } => {
                    // keep each field's current BSON type (dates stay dates, ids stay ObjectIds)
                    let current = c
                        .find_one(id.clone())
                        .await
                        .map_err(merr)?
                        .unwrap_or_default();
                    let set: Document = set
                        .into_iter()
                        .map(|(k, v)| {
                            let v = coerce_like(current.get(&k), v);
                            (k, v)
                        })
                        .collect();
                    let mut u = Document::new();
                    if !set.is_empty() {
                        u.insert("$set", set);
                    }
                    if !unset.is_empty() {
                        u.insert("$unset", unset);
                    }
                    if u.is_empty() {
                        continue;
                    }
                    total += c.update_one(id, u).await.map_err(merr)?.matched_count;
                }
                Op::Insert(d) => {
                    c.insert_one(d).await.map_err(merr)?;
                    total += 1;
                }
                Op::Delete(id) => {
                    total += c.delete_one(id).await.map_err(merr)?.deleted_count;
                }
            }
        }
        Ok(total)
    }
}

enum Op {
    Update {
        id: Document,
        set: Document,
        unset: Document,
    },
    Insert(Document),
    Delete(Document),
}

impl Op {
    fn shell(&self, coll: &str) -> String {
        let c = coll_ref(coll);
        let s = |d: &Document| shell::shell(&Bson::Document(d.clone()));
        match self {
            Op::Update { id, set, unset } => {
                let mut u = Document::new();
                if !set.is_empty() {
                    u.insert("$set", set.clone());
                }
                if !unset.is_empty() {
                    u.insert("$unset", unset.clone());
                }
                format!("db.{c}.updateOne({}, {})", s(id), s(&u))
            }
            Op::Insert(d) => format!("db.{c}.insertOne({})", s(d)),
            Op::Delete(id) => format!("db.{c}.deleteOne({})", s(id)),
        }
    }
}

fn coll_ref(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
        name.to_string()
    } else {
        format!(
            "getCollection({})",
            serde_json::to_string(name).unwrap_or_default()
        )
    }
}

fn as_i64(b: &Bson) -> Result<i64> {
    match b {
        Bson::Int32(n) => Ok(*n as i64),
        Bson::Int64(n) => Ok(*n),
        Bson::Double(n) => Ok(*n as i64),
        _ => Err(Error::Other("expected a number".into())),
    }
}

/// Statement allowed on a read-only connection?
pub fn check_read_only(s: &Stmt) -> std::result::Result<(), String> {
    match s {
        Stmt::ShowCollections | Stmt::ShowDatabases | Stmt::CollectionNames => Ok(()),
        Stmt::Command { doc, .. } => {
            let name = doc
                .keys()
                .next()
                .map(|k| k.to_lowercase())
                .unwrap_or_default();
            if READ_COMMANDS.contains(&name.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "command `{name}` is not allowed on a read-only connection"
                ))
            }
        }
        Stmt::Call { method, args, .. } => {
            if !READ_METHODS.contains(&method.as_str()) {
                return Err(format!(
                    "`{method}()` writes — not allowed on a read-only connection"
                ));
            }
            if method == "aggregate" {
                if let Some(Bson::Array(stages)) = args.first() {
                    for st in stages {
                        if let Bson::Document(d) = st {
                            if d.contains_key("$out") || d.contains_key("$merge") {
                                return Err("aggregate with $out / $merge writes — not allowed on a read-only connection".into());
                            }
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

/// `{_id: …}` for a grid row key.
fn id_filter(key: &Map<String, Value>) -> Result<Document> {
    let v = key
        .get("_id")
        .ok_or_else(|| Error::Other("row has no _id; refusing to edit".into()))?;
    Ok(doc! { "_id": id_value(v) })
}

/// Grid shows ObjectIds as their hex string; turn that back into an ObjectId.
fn id_value(v: &Value) -> Bson {
    if let Some(s) = v.as_str() {
        if let Ok(o) = ObjectId::parse_str(s) {
            return Bson::ObjectId(o);
        }
    }
    from_json(v)
}

/// Grid filters → a query document.
pub fn filter_doc(filters: &[Filter]) -> Document {
    let mut parts: Vec<Document> = Vec::new();
    for f in filters.iter().filter(|f| !f.column.is_empty()) {
        let val = |s: &str| -> Bson {
            let s = s.trim();
            if f.column == "_id" {
                if let Ok(o) = ObjectId::parse_str(s) {
                    return Bson::ObjectId(o);
                }
            }
            if f.numeric {
                if let Ok(n) = s.parse::<i64>() {
                    return if let Ok(n) = i32::try_from(n) {
                        Bson::Int32(n)
                    } else {
                        Bson::Int64(n)
                    };
                }
                if let Ok(n) = s.parse::<f64>() {
                    return Bson::Double(n);
                }
            }
            match s {
                "true" => Bson::Boolean(true),
                "false" => Bson::Boolean(false),
                _ => Bson::String(s.to_string()),
            }
        };
        let cond: Bson = match f.op {
            FilterOp::Eq => val(&f.value),
            FilterOp::Ne => doc! { "$ne": val(&f.value) }.into(),
            FilterOp::Lt => doc! { "$lt": val(&f.value) }.into(),
            FilterOp::Le => doc! { "$lte": val(&f.value) }.into(),
            FilterOp::Gt => doc! { "$gt": val(&f.value) }.into(),
            FilterOp::Ge => doc! { "$gte": val(&f.value) }.into(),
            FilterOp::Contains => doc! { "$regex": regex_escape(&f.value), "$options": "i" }.into(),
            FilterOp::StartsWith => {
                doc! { "$regex": format!("^{}", regex_escape(&f.value)), "$options": "i" }.into()
            }
            FilterOp::IsNull => Bson::Null,
            FilterOp::NotNull => doc! { "$ne": Bson::Null }.into(),
            FilterOp::In => {
                let items: Vec<Bson> = f
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(val)
                    .collect();
                doc! { "$in": items }.into()
            }
        };
        parts.push(doc! { f.column.clone(): cond });
    }
    match parts.len() {
        0 => Document::new(),
        1 => parts.pop().unwrap_or_default(),
        _ => doc! { "$and": parts },
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn type_name(v: &Bson) -> &'static str {
    match v {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Document(_) => "object",
        Bson::Array(_) => "array",
        Bson::Binary(_) => "binData",
        Bson::ObjectId(_) => "objectId",
        Bson::Boolean(_) => "bool",
        Bson::DateTime(_) => "date",
        Bson::Null => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::Int32(_) => "int",
        Bson::Int64(_) => "long",
        Bson::Timestamp(_) => "timestamp",
        Bson::Decimal128(_) => "decimal",
        _ => "other",
    }
}

/// BSON → JSON for the grid (ObjectIds and dates as strings).
pub fn to_json(v: &Bson) -> Value {
    match v {
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Boolean(b) => Value::Bool(*b),
        Bson::Int32(n) => Value::from(*n),
        Bson::Int64(n) => Value::from(*n),
        Bson::Double(n) => Number::from_f64(*n)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(n.to_string())),
        Bson::String(s) => Value::String(s.clone()),
        Bson::ObjectId(o) => Value::String(o.to_hex()),
        Bson::DateTime(d) => Value::String(
            d.try_to_rfc3339_string()
                .unwrap_or_else(|_| d.timestamp_millis().to_string()),
        ),
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Document(d) => {
            Value::Object(d.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
        Bson::Array(a) => Value::Array(a.iter().map(to_json).collect()),
        Bson::Binary(b) if b.subtype == bson::spec::BinarySubtype::Uuid && b.bytes.len() == 16 => {
            let h: String = b.bytes.iter().map(|x| format!("{x:02x}")).collect();
            Value::String(format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            ))
        }
        other => Value::String(shell::shell(other)),
    }
}

/// JSON from the grid → BSON. Strings that hold a JSON object/array become documents.
pub fn from_json(v: &Value) -> Bson {
    match v {
        Value::Null => Bson::Null,
        Value::Bool(b) => Bson::Boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i32::try_from(i) {
                    Ok(i) => Bson::Int32(i),
                    Err(_) => Bson::Int64(i),
                }
            } else {
                Bson::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => {
            let t = s.trim_start();
            if t.starts_with('{') || t.starts_with('[') {
                if let Ok(b) = shell::parse_value(s) {
                    return b;
                }
            }
            Bson::String(s.clone())
        }
        Value::Array(a) => Bson::Array(a.iter().map(from_json).collect()),
        Value::Object(o) => {
            Bson::Document(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect())
        }
    }
}

/// Keep the stored type when the grid sends back a string/number.
pub fn coerce_like(current: Option<&Bson>, new: Bson) -> Bson {
    let Some(cur) = current else { return new };
    let text = match &new {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        _ => return new,
    };
    let t = text.trim();
    let coerced = match cur {
        Bson::DateTime(_) => shell::parse_date(t).map(Bson::DateTime),
        Bson::ObjectId(_) => ObjectId::parse_str(t).ok().map(Bson::ObjectId),
        Bson::Int32(_) => t.parse::<i32>().ok().map(Bson::Int32),
        Bson::Int64(_) => t.parse::<i64>().ok().map(Bson::Int64),
        Bson::Double(_) => t.parse::<f64>().ok().map(Bson::Double),
        Bson::Decimal128(_) => t.parse::<bson::Decimal128>().ok().map(Bson::Decimal128),
        Bson::Boolean(_) => match t {
            "true" | "1" => Some(Bson::Boolean(true)),
            "false" | "0" => Some(Bson::Boolean(false)),
            _ => None,
        },
        _ => None,
    };
    coerced.unwrap_or(new)
}

fn single_column(name: &str, vals: impl IntoIterator<Item = Value>) -> QueryResult {
    QueryResult {
        columns: vec![ResultColumn {
            name: name.into(),
            data_type: String::new(),
        }],
        rows: vals.into_iter().map(|v| vec![v]).collect(),
        rows_affected: 0,
        truncated: false,
    }
}

/// Documents → grid: one column per top-level field (union, first-seen order, `_id` first).
pub fn docs_result(mut docs: Vec<Document>, limit: usize) -> QueryResult {
    let truncated = docs.len() > limit;
    docs.truncate(limit);
    let mut cols: Vec<(String, &'static str)> = Vec::new();
    if docs.iter().any(|d| d.contains_key("_id")) {
        cols.push(("_id".into(), ""));
    }
    for d in &docs {
        for (k, v) in d {
            match cols.iter_mut().find(|c| &c.0 == k) {
                Some(c) => {
                    if c.1.is_empty() && !matches!(v, Bson::Null) {
                        c.1 = type_name(v);
                    }
                }
                None => cols.push((
                    k.clone(),
                    if matches!(v, Bson::Null) {
                        ""
                    } else {
                        type_name(v)
                    },
                )),
            }
        }
    }
    let rows = docs
        .iter()
        .map(|d| {
            cols.iter()
                .map(|(k, _)| d.get(k).map(to_json).unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    QueryResult {
        columns: cols
            .into_iter()
            .map(|(name, t)| ResultColumn {
                name,
                data_type: t.into(),
            })
            .collect(),
        rows,
        rows_affected: 0,
        truncated,
    }
}
