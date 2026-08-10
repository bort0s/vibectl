//! Stack detectors: what a project is written in and with.
//!
//! Every one of these follows the same discipline. A version that the
//! authoritative manifest states directly is `Certain`. A framework inferred
//! from a config file with no corresponding dependency is `Likely`. Anything
//! resting on a file that is not *about* the fact — a Dockerfile base image, a
//! stray `.py` — is `Weak` and therefore never reaches disk.

use serde::Deserialize;

use crate::detect::merge::text_finding;
use crate::detect::{
    DetectCtx, DetectError, Detector, DetectorId, Evidence, FieldPath, Finding, Interest,
    Specificity,
};
use crate::model::Confidence;

/// Frameworks worth naming, keyed by the dependency that proves them.
///
/// A fixed list rather than "every dependency", because a manifest listing all
/// 900 transitive packages is not a stack summary. The cost is that a framework
/// nobody added here is invisible; the mitigation is that adding one is a line.
const NODE_FRAMEWORKS: &[(&str, &str)] = &[
    ("react", "react"),
    ("next", "next"),
    ("vue", "vue"),
    ("nuxt", "nuxt"),
    ("svelte", "svelte"),
    ("@sveltejs/kit", "sveltekit"),
    ("solid-js", "solid"),
    ("astro", "astro"),
    ("angular", "angular"),
    ("@angular/core", "angular"),
    ("express", "express"),
    ("fastify", "fastify"),
    ("hono", "hono"),
    ("nest", "nest"),
    ("@nestjs/core", "nest"),
    ("vite", "vite"),
    ("webpack", "webpack"),
    ("typescript", "typescript"),
    ("tailwindcss", "tailwind"),
    ("electron", "electron"),
    ("@tauri-apps/api", "tauri"),
];

/// Hosted services worth recording, keyed by the dependency that proves them.
const NODE_SERVICES: &[(&str, &str)] = &[
    ("@supabase/supabase-js", "supabase"),
    ("firebase", "firebase"),
    ("@planetscale/database", "planetscale"),
    ("@vercel/analytics", "vercel"),
    ("@neondatabase/serverless", "neon"),
    ("stripe", "stripe"),
    ("@clerk/nextjs", "clerk"),
    ("@auth0/auth0-react", "auth0"),
    ("@sentry/node", "sentry"),
    ("@sentry/react", "sentry"),
    ("redis", "redis"),
    ("ioredis", "redis"),
    ("pg", "postgres"),
    ("mongodb", "mongodb"),
];

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    engines: Option<Engines>,
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Engines {
    #[serde(default)]
    node: Option<String>,
}

#[derive(Debug)]
pub struct NodePackageJson;

const NODE_ID: DetectorId = DetectorId("node.package_json");

impl Detector for NodePackageJson {
    fn id(&self) -> DetectorId {
        NODE_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("package.json")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[
            FieldPath::ProjectDescription,
            FieldPath::StackRuntime,
            FieldPath::StackFramework,
            FieldPath::StackService,
        ]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "package.json";
        let text = ctx.read_text(rel)?;
        let pkg: PackageJson =
            serde_json::from_str(&text).map_err(|e| DetectError::Unreadable {
                path: rel.to_owned(),
                why: e.to_string(),
            })?;

        let path = std::path::Path::new(rel);
        let mut out = Vec::new();

        if let Some(desc) = pkg.description.as_deref().map(str::trim) {
            if !desc.is_empty() {
                out.push(text_finding(
                    FieldPath::ProjectDescription,
                    desc,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, "$.description", desc),
                    NODE_ID,
                ));
            }
        }

        // `engines.node` is a *range* — ">=20", "^22.1.0" — so only its major
        // is claimed; inventing a patch level the file does not contain would
        // be fake precision.
        //
        // The claim is `Certain`, not `Likely`. Confidence answers "does the
        // authoritative file state this?", and `package.json` stating
        // `engines.node` is exactly that. Downgrading it here would conflate
        // *approximate version* with *uncertain language*, and the consequence
        // is not cosmetic: a polyglot repo would then resolve silently in
        // favour of whichever language happened to be claimed at a higher
        // level, instead of being reported as the conflict it is. The
        // approximation is carried by `Specificity::Manifest`.
        //
        // Known limitation, stated because the previous version of this comment
        // claimed a backstop that does not exist: no detector currently emits
        // `StackRuntime` at `Specificity::Lockfile`, so nothing corrects this
        // value downward. `engines.node` is a compatibility *floor* advertised
        // to consumers, not the runtime in use, and a library holding `>=14`
        // while its authors run 22 will be reported as `node@14`. Reading the
        // resolved version out of four lockfile formats is what fixes it
        // properly; until then this is the most authoritative statement the
        // repository makes about itself.
        if let Some(range) = pkg.engines.as_ref().and_then(|e| e.node.as_deref()) {
            if let Some(major) = major_from_range(range) {
                out.push(text_finding(
                    FieldPath::StackRuntime,
                    format!("node@{major}"),
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, "$.engines.node", range),
                    NODE_ID,
                ));
            }
        }

        let all_deps: Vec<(&String, &String)> = pkg
            .dependencies
            .iter()
            .chain(pkg.dev_dependencies.iter())
            .collect();

        for (name, version) in &all_deps {
            if let Some((_, label)) = NODE_FRAMEWORKS.iter().find(|(dep, _)| dep == name) {
                let value = match version_label(version) {
                    Some(v) => format!("{label}@{v}"),
                    None => (*label).to_owned(),
                };
                out.push(text_finding(
                    FieldPath::StackFramework,
                    value,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, format!("$.dependencies.{name}"), (*version).clone()),
                    NODE_ID,
                ));
            }
            if let Some((_, label)) = NODE_SERVICES.iter().find(|(dep, _)| dep == name) {
                out.push(text_finding(
                    FieldPath::StackService,
                    *label,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, format!("$.dependencies.{name}"), (*version).clone()),
                    NODE_ID,
                ));
            }
        }

        // A package.json with no engines field still proves the runtime is
        // Node. It does not prove *which* Node, and this does not pretend to:
        // no version is attached.
        if !out.iter().any(|f| f.field == FieldPath::StackRuntime) {
            out.push(text_finding(
                FieldPath::StackRuntime,
                "node",
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$", "a package.json exists"),
                NODE_ID,
            ));
        }

        Ok(out)
    }
}

/// The major version from a semver range, if one is unambiguously stated.
///
/// `>=20` and `^22.1.0` yield `20` and `22`. A range with no clear floor —
/// `*`, `latest`, an empty string — yields nothing, because there is nothing
/// honest to say about it.
fn major_from_range(range: &str) -> Option<u32> {
    let digits: String = range
        .trim_start_matches(['^', '~', '>', '=', '<', 'v', ' '])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// A displayable version from a dependency specifier, if there is one.
fn version_label(spec: &str) -> Option<u32> {
    major_from_range(spec)
}

#[derive(Debug)]
pub struct CargoToml;

const CARGO_ID: DetectorId = DetectorId("rust.cargo_toml");

impl Detector for CargoToml {
    fn id(&self) -> DetectorId {
        CARGO_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("Cargo.toml")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[
            FieldPath::ProjectDescription,
            FieldPath::StackRuntime,
            FieldPath::StackFramework,
        ]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "Cargo.toml";
        let text = ctx.read_text(rel)?;
        let doc: toml_edit::DocumentMut =
            text.parse()
                .map_err(|e: toml_edit::TomlError| DetectError::Unreadable {
                    path: rel.to_owned(),
                    why: e.to_string(),
                })?;

        let path = std::path::Path::new(rel);
        let mut out = Vec::new();

        // A workspace root with no [package] is still a Rust project.
        let package = doc.get("package").and_then(|p| p.as_table());

        if let Some(desc) = package
            .and_then(|p| p.get("description"))
            .and_then(|d| d.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            out.push(text_finding(
                FieldPath::ProjectDescription,
                desc,
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "package.description", desc),
                CARGO_ID,
            ));
        }

        let runtime = match package
            .and_then(|p| p.get("rust-version"))
            .and_then(|v| v.as_str())
        {
            Some(v) => (
                format!("rust@{v}"),
                Confidence::Certain,
                "package.rust-version",
                v.to_owned(),
            ),
            None => (
                "rust".to_owned(),
                Confidence::Certain,
                "$",
                "a Cargo.toml exists".to_owned(),
            ),
        };
        out.push(text_finding(
            FieldPath::StackRuntime,
            runtime.0,
            runtime.1,
            Specificity::Manifest,
            Evidence::from_file(path, runtime.2, runtime.3),
            CARGO_ID,
        ));

        const RUST_FRAMEWORKS: &[&str] = &[
            "axum",
            "actix-web",
            "rocket",
            "tokio",
            "bevy",
            "tauri",
            "leptos",
            "dioxus",
            "clap",
            "serde",
            "diesel",
            "sqlx",
        ];
        if let Some(deps) = doc.get("dependencies").and_then(|d| d.as_table()) {
            for (name, _) in deps.iter() {
                if RUST_FRAMEWORKS.contains(&name) {
                    out.push(text_finding(
                        FieldPath::StackFramework,
                        name,
                        Confidence::Certain,
                        Specificity::Manifest,
                        Evidence::from_file(path, format!("dependencies.{name}"), name),
                        CARGO_ID,
                    ));
                }
            }
        }

        Ok(out)
    }
}

#[derive(Debug)]
pub struct PyProject;

const PY_ID: DetectorId = DetectorId("python.pyproject");

impl Detector for PyProject {
    fn id(&self) -> DetectorId {
        PY_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("pyproject.toml")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[
            FieldPath::ProjectDescription,
            FieldPath::StackRuntime,
            FieldPath::StackFramework,
        ]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "pyproject.toml";
        let text = ctx.read_text(rel)?;
        let doc: toml_edit::DocumentMut =
            text.parse()
                .map_err(|e: toml_edit::TomlError| DetectError::Unreadable {
                    path: rel.to_owned(),
                    why: e.to_string(),
                })?;

        let path = std::path::Path::new(rel);
        let mut out = Vec::new();
        let project = doc.get("project").and_then(|p| p.as_table());

        if let Some(desc) = project
            .and_then(|p| p.get("description"))
            .and_then(|d| d.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            out.push(text_finding(
                FieldPath::ProjectDescription,
                desc,
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "project.description", desc),
                PY_ID,
            ));
        }

        match project
            .and_then(|p| p.get("requires-python"))
            .and_then(|v| v.as_str())
        {
            Some(range) => {
                let label = python_version(range)
                    .map_or_else(|| "python".to_owned(), |v| format!("python@{v}"));
                out.push(text_finding(
                    FieldPath::StackRuntime,
                    label,
                    // Certain for the same reason as `engines.node`: the
                    // authoritative manifest states it. See NodePackageJson.
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, "project.requires-python", range),
                    PY_ID,
                ));
            }
            None => out.push(text_finding(
                FieldPath::StackRuntime,
                "python",
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$", "a pyproject.toml exists"),
                PY_ID,
            )),
        }

        // Parse `project.dependencies` rather than scanning the whole file.
        // A substring search matched `pytest` inside `[tool.pytest.ini_options]`
        // and `pydantic` inside `plugins = ["pydantic.mypy"]`, then cited both
        // to `project.dependencies` — a table containing neither. A citation
        // that points somewhere the value is not is worse than no citation,
        // because it survives review.
        let deps = doc
            .get("project")
            .and_then(|p| p.get("dependencies"))
            .and_then(toml_edit::Item::as_array);
        if let Some(deps) = deps {
            for entry in deps.iter().filter_map(toml_edit::Value::as_str) {
                let Some(name) = requirement_name(entry) else {
                    continue;
                };
                if let Some((_, label)) = PY_FRAMEWORKS.iter().find(|(dep, _)| *dep == name) {
                    out.push(text_finding(
                        FieldPath::StackFramework,
                        *label,
                        Confidence::Certain,
                        Specificity::Manifest,
                        Evidence::from_file(path, "project.dependencies", entry),
                        PY_ID,
                    ));
                }
            }
        }

        Ok(out)
    }
}

/// The bare package name from a PEP 508 requirement: `fastapi[all]>=0.110` and
/// `Django ~= 5.0 ; python_version < "3.12"` both yield their package name.
fn requirement_name(spec: &str) -> Option<&str> {
    let name = spec
        .trim()
        .split([' ', '=', '>', '<', '!', '~', '[', ';', ',', '('])
        .next()?
        .trim();
    if name.is_empty() || name.starts_with('#') {
        None
    } else {
        Some(name)
    }
}

const PY_FRAMEWORKS: &[(&str, &str)] = &[
    ("fastapi", "fastapi"),
    ("django", "django"),
    ("flask", "flask"),
    ("pydantic", "pydantic"),
    ("sqlalchemy", "sqlalchemy"),
    ("pytest", "pytest"),
    ("numpy", "numpy"),
    ("pandas", "pandas"),
    ("torch", "pytorch"),
    ("streamlit", "streamlit"),
];

/// `>=3.11` and `^3.12` yield `3.11` and `3.12`.
fn python_version(range: &str) -> Option<String> {
    let cleaned = range.trim_start_matches(['>', '=', '<', '~', '^', ' ']);
    let mut parts = cleaned.split('.');
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor: u32 = parts.next().and_then(|m| {
        m.trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok()
    })?;
    Some(format!("{major}.{minor}"))
}

#[derive(Debug)]
pub struct PyRequirements;

const PYREQ_ID: DetectorId = DetectorId("python.requirements");

impl Detector for PyRequirements {
    fn id(&self) -> DetectorId {
        PYREQ_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileNamePattern {
            prefix: "requirements",
            suffix: ".txt",
        }]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackRuntime, FieldPath::StackFramework]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let paths: Vec<String> = ctx
            .files
            .root_files_matching("requirements", ".txt")
            .into_iter()
            .map(str::to_owned)
            .collect();

        let mut out = Vec::new();
        for rel in &paths {
            let Ok(text) = ctx.read_text(rel) else {
                continue;
            };
            let path = std::path::Path::new(rel);

            // A requirements.txt proves Python is in use. It says nothing at
            // all about which version, and this does not invent one.
            out.push(text_finding(
                FieldPath::StackRuntime,
                "python",
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$", "a requirements file exists"),
                PYREQ_ID,
            ));

            let lower = text.to_lowercase();
            for (name, label) in PY_FRAMEWORKS {
                if lower.lines().any(|l| {
                    let l = l.trim();
                    !l.starts_with('#')
                        && l.split(['=', '>', '<', '[', ';', ' ']).next() == Some(name)
                }) {
                    out.push(text_finding(
                        FieldPath::StackFramework,
                        *label,
                        // No version pinning guarantee, so Likely rather than
                        // Certain, per ADR-0003 §7.
                        Confidence::Likely,
                        Specificity::Manifest,
                        Evidence::from_file(path, "line", *name),
                        PYREQ_ID,
                    ));
                }
            }
        }
        Ok(out)
    }
}

#[derive(Debug)]
pub struct GoMod;

const GO_ID: DetectorId = DetectorId("go.mod");

impl Detector for GoMod {
    fn id(&self) -> DetectorId {
        GO_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("go.mod")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackRuntime, FieldPath::StackFramework]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "go.mod";
        let text = ctx.read_text(rel)?;
        let path = std::path::Path::new(rel);
        let mut out = Vec::new();

        // The `go` directive states the language version directly — that is
        // what makes this Certain where a semver range is only Likely.
        let go_line = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("go ") && !l.starts_with("go.mod"));
        match go_line.and_then(|l| l.split_whitespace().nth(1)) {
            Some(v) => out.push(text_finding(
                FieldPath::StackRuntime,
                format!("go@{v}"),
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "go directive", go_line.unwrap_or_default()),
                GO_ID,
            )),
            None => out.push(text_finding(
                FieldPath::StackRuntime,
                "go",
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$", "a go.mod exists"),
                GO_ID,
            )),
        }

        const GO_FRAMEWORKS: &[(&str, &str)] = &[
            ("github.com/gin-gonic/gin", "gin"),
            ("github.com/labstack/echo", "echo"),
            ("github.com/gofiber/fiber", "fiber"),
            ("github.com/go-chi/chi", "chi"),
            ("gorm.io/gorm", "gorm"),
            ("github.com/spf13/cobra", "cobra"),
        ];
        for (module, label) in GO_FRAMEWORKS {
            if text.contains(module) {
                out.push(text_finding(
                    FieldPath::StackFramework,
                    *label,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, "require", *module),
                    GO_ID,
                ));
            }
        }

        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct ComposerJson {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    require: std::collections::BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct ComposerJsonDetector;

const PHP_ID: DetectorId = DetectorId("php.composer");

impl Detector for ComposerJsonDetector {
    fn id(&self) -> DetectorId {
        PHP_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("composer.json")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[
            FieldPath::ProjectDescription,
            FieldPath::StackRuntime,
            FieldPath::StackFramework,
        ]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "composer.json";
        let text = ctx.read_text(rel)?;
        let pkg: ComposerJson =
            serde_json::from_str(&text).map_err(|e| DetectError::Unreadable {
                path: rel.to_owned(),
                why: e.to_string(),
            })?;

        let path = std::path::Path::new(rel);
        let mut out = Vec::new();

        if let Some(desc) = pkg
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            out.push(text_finding(
                FieldPath::ProjectDescription,
                desc,
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$.description", desc),
                PHP_ID,
            ));
        }

        match pkg.require.get("php") {
            Some(range) => {
                let label =
                    python_version(range).map_or_else(|| "php".to_owned(), |v| format!("php@{v}"));
                out.push(text_finding(
                    FieldPath::StackRuntime,
                    label,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, "$.require.php", range),
                    PHP_ID,
                ));
            }
            None => out.push(text_finding(
                FieldPath::StackRuntime,
                "php",
                Confidence::Certain,
                Specificity::Manifest,
                Evidence::from_file(path, "$", "a composer.json exists"),
                PHP_ID,
            )),
        }

        const PHP_FRAMEWORKS: &[(&str, &str)] = &[
            ("laravel/framework", "laravel"),
            ("symfony/symfony", "symfony"),
            ("drupal/core", "drupal"),
            ("livewire/livewire", "livewire"),
            ("filament/filament", "filament"),
            ("slim/slim", "slim"),
        ];
        for (dep, label) in PHP_FRAMEWORKS {
            if pkg.require.contains_key(*dep) {
                out.push(text_finding(
                    FieldPath::StackFramework,
                    *label,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_file(path, format!("$.require.{dep}"), *dep),
                    PHP_ID,
                ));
            }
        }

        Ok(out)
    }
}

/// Node lockfiles, which record what is actually resolved rather than what was
/// asked for. Outranks `package.json` on specificity for that reason.
#[derive(Debug)]
pub struct NodeLockfile;

const LOCK_ID: DetectorId = DetectorId("node.lockfile");

impl Detector for NodeLockfile {
    fn id(&self) -> DetectorId {
        LOCK_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[
            Interest::FileName("package-lock.json"),
            Interest::FileName("pnpm-lock.yaml"),
            Interest::FileName("yarn.lock"),
            Interest::FileName("bun.lockb"),
        ]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackService]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        // Which package manager is in use is a fact the lockfile states
        // directly, and it is the only thing this detector claims. Parsing
        // resolved versions out of four different lockfile formats is real
        // work for a small gain, and guessing at them would be worse than not
        // claiming them — so it does not.
        let managers = [
            ("package-lock.json", "npm"),
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("bun.lockb", "bun"),
        ];
        let mut out = Vec::new();
        for (file, label) in managers {
            // `has_file` is root-level, so a lockfile belonging to a nested
            // example or fixture directory no longer makes a claim about this
            // project. A Rust crate with `examples/wasm-demo/package-lock.json`
            // was reporting `npm` as one of its services, cited to a
            // root-level lockfile that did not exist.
            if ctx.files.has_file(file) {
                out.push(text_finding(
                    FieldPath::StackService,
                    label,
                    Confidence::Certain,
                    Specificity::Lockfile,
                    Evidence::from_file(std::path::Path::new(file), "$", "lockfile present"),
                    LOCK_ID,
                ));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_ranges_yield_a_major_or_nothing() {
        assert_eq!(major_from_range(">=20"), Some(20));
        assert_eq!(major_from_range("^22.1.0"), Some(22));
        assert_eq!(major_from_range("~18"), Some(18));
        assert_eq!(major_from_range("v20.1"), Some(20));

        // Nothing honest to say about these.
        assert_eq!(major_from_range("*"), None);
        assert_eq!(major_from_range("latest"), None);
        assert_eq!(major_from_range(""), None);
    }

    #[test]
    fn python_ranges_yield_major_minor_or_nothing() {
        assert_eq!(python_version(">=3.11"), Some("3.11".to_owned()));
        assert_eq!(python_version("^3.12"), Some("3.12".to_owned()));
        assert_eq!(python_version("3"), None, "a major alone is not a version");
        assert_eq!(python_version("*"), None);
    }
}
