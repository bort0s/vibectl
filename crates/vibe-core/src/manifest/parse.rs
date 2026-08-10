//! `DocumentMut` -> [`Manifest`]. Tolerant where tolerance is honest, strict
//! where guessing would be dishonest.
//!
//! Tolerant: a missing table, a missing optional key, an unrecognised `status`
//! value, an unparseable `created` date, and any key this build has never heard
//! of. All of these are read as best we can and preserved verbatim on write.
//!
//! Strict: a known key holding the wrong TOML type. Within one major version a
//! key's type is part of its meaning, so `runtime = 42` is a broken file rather
//! than a forward-compatible one — and inventing a plausible value for it would
//! violate the "never guess" constraint more seriously than refusing does.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::error::CoreError;
use crate::manifest::document::item_kind_name;
use crate::manifest::version::{Compat, SCHEMA_MAJOR, SchemaVersion};
use crate::model::{
    Context, Deploy, Manifest, Project, Repo, Stack, Status, UnknownKey, ValueKind, Visibility,
};

const TOP_LEVEL_KEYS: &[&str] = &[
    "schema_version",
    "project",
    "stack",
    "repo",
    "deploy",
    "context",
];
const PROJECT_KEYS: &[&str] = &["name", "description", "status", "archived", "created"];
const STACK_KEYS: &[&str] = &["runtime", "frameworks", "services"];
const REPO_KEYS: &[&str] = &["remote", "visibility"];
const DEPLOY_KEYS: &[&str] = &["url", "env_required"];
const CONTEXT_KEYS: &[&str] = &["decisions", "next"];

pub(crate) fn parse(path: &Path, doc: &DocumentMut) -> Result<Manifest, CoreError> {
    let schema_version = read_schema_version(path, doc)?;

    if schema_version.compat() == Compat::MajorMismatch {
        return Err(CoreError::SchemaMajorMismatch {
            path: path.to_path_buf(),
            found: schema_version,
            supported_major: SCHEMA_MAJOR,
        });
    }

    let mut unknown = Vec::new();
    collect_unknown(doc, "", TOP_LEVEL_KEYS, &mut unknown);

    let project_tbl = table(path, doc, "project")?;
    let stack_tbl = table(path, doc, "stack")?;
    let repo_tbl = table(path, doc, "repo")?;
    let deploy_tbl = table(path, doc, "deploy")?;
    let context_tbl = table(path, doc, "context")?;

    for (tbl, name, known) in [
        (project_tbl, "project", PROJECT_KEYS),
        (stack_tbl, "stack", STACK_KEYS),
        (repo_tbl, "repo", REPO_KEYS),
        (deploy_tbl, "deploy", DEPLOY_KEYS),
        (context_tbl, "context", CONTEXT_KEYS),
    ] {
        if let Some(t) = tbl {
            collect_unknown(t, name, known, &mut unknown);
        }
    }

    let name = opt_str(path, project_tbl, "project", "name")?.ok_or_else(|| {
        CoreError::ManifestMissingField {
            path: path.to_path_buf(),
            field: "project.name".to_owned(),
        }
    })?;

    let project = Project {
        name,
        description: opt_str(path, project_tbl, "project", "description")?,
        // Never fails: an unrecognised value becomes Status::Other and is
        // written back verbatim only if someone explicitly changes it.
        status: opt_str(path, project_tbl, "project", "status")?
            .map_or(Status::Idea, |s| s.parse().unwrap_or(Status::Idea)),
        archived: opt_bool(path, project_tbl, "project", "archived")?.unwrap_or(false),
        created: opt_str(path, project_tbl, "project", "created")?,
    };

    let stack = Stack {
        runtime: opt_str(path, stack_tbl, "stack", "runtime")?,
        frameworks: opt_str_array(path, stack_tbl, "stack", "frameworks")?,
        services: opt_str_array(path, stack_tbl, "stack", "services")?,
    };

    let repo = Repo {
        remote: opt_str(path, repo_tbl, "repo", "remote")?,
        visibility: opt_str(path, repo_tbl, "repo", "visibility")?
            .map(|s| s.parse::<Visibility>().unwrap_or(Visibility::Other(s))),
    };

    let deploy = Deploy {
        url: opt_str(path, deploy_tbl, "deploy", "url")?,
        env_required: opt_str_array(path, deploy_tbl, "deploy", "env_required")?,
    };

    let context = Context {
        decisions: opt_str_array(path, context_tbl, "context", "decisions")?,
        next: opt_str_array(path, context_tbl, "context", "next")?,
    };

    Ok(Manifest {
        schema_version,
        project,
        stack,
        repo,
        deploy,
        context,
        unknown,
    })
}

fn read_schema_version(path: &Path, doc: &DocumentMut) -> Result<SchemaVersion, CoreError> {
    let Some(item) = doc.get("schema_version") else {
        // Absent means 1.0, not an error. Hand-written and pre-versioning
        // manifests must work (ADR-0002 §4).
        return Ok(SchemaVersion::default());
    };
    let Some(s) = item.as_str() else {
        return Err(CoreError::SchemaVersionUnreadable {
            path: path.to_path_buf(),
            found: item.to_string().trim().to_owned(),
        });
    };
    s.parse().map_err(|_| CoreError::SchemaVersionUnreadable {
        path: path.to_path_buf(),
        found: s.to_owned(),
    })
}

fn table<'a>(
    path: &Path,
    doc: &'a DocumentMut,
    name: &'static str,
) -> Result<Option<&'a Table>, CoreError> {
    match doc.get(name) {
        None | Some(Item::None) => Ok(None),
        Some(Item::Table(t)) => Ok(Some(t)),
        Some(other) => Err(CoreError::ManifestFieldType {
            path: path.to_path_buf(),
            field: name.to_owned(),
            expected: "a table",
            found: item_kind_name(other),
        }),
    }
}

fn opt_str(
    path: &Path,
    tbl: Option<&Table>,
    table_name: &str,
    key: &str,
) -> Result<Option<String>, CoreError> {
    let Some(item) = tbl.and_then(|t| t.get(key)) else {
        return Ok(None);
    };
    match item.as_str() {
        // An empty string means "not determined". It is stored rather than
        // omitted so the key stays visible in the file as a slot the user or a
        // later `vibe sync` can fill — but it reads as absent.
        Some("") => Ok(None),
        Some(s) => Ok(Some(s.to_owned())),
        None => Err(CoreError::ManifestFieldType {
            path: path.to_path_buf(),
            field: format!("{table_name}.{key}"),
            expected: "a string",
            found: item_kind_name(item),
        }),
    }
}

fn opt_bool(
    path: &Path,
    tbl: Option<&Table>,
    table_name: &str,
    key: &str,
) -> Result<Option<bool>, CoreError> {
    let Some(item) = tbl.and_then(|t| t.get(key)) else {
        return Ok(None);
    };
    item.as_bool()
        .map(Some)
        .ok_or_else(|| CoreError::ManifestFieldType {
            path: path.to_path_buf(),
            field: format!("{table_name}.{key}"),
            expected: "a boolean",
            found: item_kind_name(item),
        })
}

fn opt_str_array(
    path: &Path,
    tbl: Option<&Table>,
    table_name: &str,
    key: &str,
) -> Result<Vec<String>, CoreError> {
    let Some(item) = tbl.and_then(|t| t.get(key)) else {
        return Ok(Vec::new());
    };
    let field = || format!("{table_name}.{key}");
    let arr = item
        .as_array()
        .ok_or_else(|| CoreError::ManifestFieldType {
            path: path.to_path_buf(),
            field: field(),
            expected: "an array of strings",
            found: item_kind_name(item),
        })?;

    arr.iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| CoreError::ManifestFieldType {
                    path: path.to_path_buf(),
                    field: field(),
                    expected: "an array of strings",
                    found: value_kind_name(v),
                })
        })
        .collect()
}

fn collect_unknown(tbl: &Table, prefix: &str, known: &[&str], out: &mut Vec<UnknownKey>) {
    for (key, item) in tbl.iter() {
        if known.contains(&key) {
            continue;
        }
        let dotted_path = if prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{prefix}.{key}")
        };
        // An unknown *table* is recorded as one entry, not recursed into.
        // "this build does not understand `ai`" is the useful report; listing
        // `ai.model`, `ai.summary`, `ai.generated_at` separately is noise.
        out.push(UnknownKey {
            dotted_path,
            kind: kind_of(item),
        });
    }
}

fn kind_of(item: &Item) -> ValueKind {
    match item {
        Item::Table(_) => ValueKind::Table,
        Item::ArrayOfTables(_) => ValueKind::Array,
        Item::None => ValueKind::Table,
        Item::Value(v) => match v {
            Value::String(_) => ValueKind::String,
            Value::Integer(_) => ValueKind::Integer,
            Value::Float(_) => ValueKind::Float,
            Value::Boolean(_) => ValueKind::Boolean,
            Value::Datetime(_) => ValueKind::Datetime,
            Value::Array(_) => ValueKind::Array,
            Value::InlineTable(_) => ValueKind::Table,
        },
    }
}

fn value_kind_name(v: &Value) -> &'static str {
    item_kind_name(&Item::Value(v.clone()))
}
