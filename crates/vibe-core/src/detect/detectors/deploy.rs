//! Deploy targets, and the environment variables a project needs.
//!
//! Every URL here is `Likely`, never `Certain`. A `vercel.json` proves the
//! project is *configured* for Vercel; it does not prove anything is deployed,
//! and a URL derived from a project name is a construction, not an observation.
//! Recording that distinction is the difference between a registry a user
//! trusts and one that lists dead links with total confidence.

use crate::detect::merge::text_finding;
use crate::detect::{
    DetectCtx, DetectError, Detector, DetectorId, Evidence, FieldPath, Finding, Interest,
    Specificity,
};
use crate::model::Confidence;

#[derive(Debug)]
pub struct VercelConfig;

const VERCEL_ID: DetectorId = DetectorId("deploy.vercel");

impl Detector for VercelConfig {
    fn id(&self) -> DetectorId {
        VERCEL_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[
            Interest::FileName("vercel.json"),
            Interest::DirName(".vercel"),
        ]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackService, FieldPath::DeployUrl]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let source = if ctx.files.has_file("vercel.json") {
            "vercel.json"
        } else {
            ".vercel"
        };
        let mut out = vec![text_finding(
            FieldPath::StackService,
            "vercel",
            Confidence::Certain,
            Specificity::Config,
            Evidence::from_file(std::path::Path::new(source), "$", "vercel config present"),
            VERCEL_ID,
        )];

        // `.vercel/project.json` records the linked project name. The
        // conventional URL derived from it is a *guess about hosting*, so it
        // is Likely — the file does not say the site is live at that address.
        if ctx.files.contains_path(".vercel/project.json") {
            if let Ok(text) = ctx.read_text(".vercel/project.json") {
                if let Some(name) = json_string_field(&text, "name") {
                    out.push(text_finding(
                        FieldPath::DeployUrl,
                        format!("https://{name}.vercel.app"),
                        Confidence::Likely,
                        Specificity::Config,
                        Evidence::from_file(
                            std::path::Path::new(".vercel/project.json"),
                            "$.name",
                            name,
                        ),
                        VERCEL_ID,
                    ));
                }
            }
        }
        Ok(out)
    }
}

#[derive(Debug)]
pub struct NetlifyConfig;

const NETLIFY_ID: DetectorId = DetectorId("deploy.netlify");

impl Detector for NetlifyConfig {
    fn id(&self) -> DetectorId {
        NETLIFY_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[
            Interest::FileName("netlify.toml"),
            Interest::DirName(".netlify"),
        ]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackService, FieldPath::DeployUrl]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        // Cite whichever file actually matched. Emitting a fixed
        // `netlify.toml` citation meant a project detected via `.netlify/`
        // alone was justified by a file that is not in the directory — a
        // citation that survives review precisely because it looks right.
        let source = if ctx.files.has_file("netlify.toml") {
            "netlify.toml"
        } else {
            ".netlify"
        };
        // Deliberately no URL. `netlify.toml` does not record the site name —
        // that lives in `.netlify/state.json`, which is gitignored and usually
        // absent. Constructing `https://<directory-name>.netlify.app` would be
        // inventing an address from a directory name, which is exactly the
        // failure mode this project exists to avoid.
        Ok(vec![text_finding(
            FieldPath::StackService,
            "netlify",
            Confidence::Certain,
            Specificity::Config,
            Evidence::from_file(std::path::Path::new(source), "$", "netlify config present"),
            NETLIFY_ID,
        )])
    }
}

#[derive(Debug)]
pub struct FlyConfig;

const FLY_ID: DetectorId = DetectorId("deploy.fly");

impl Detector for FlyConfig {
    fn id(&self) -> DetectorId {
        FLY_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::FileName("fly.toml")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::StackService, FieldPath::DeployUrl]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let rel = "fly.toml";
        let text = ctx.read_text(rel)?;
        let path = std::path::Path::new(rel);

        let mut out = vec![text_finding(
            FieldPath::StackService,
            "fly",
            Confidence::Certain,
            Specificity::Config,
            Evidence::from_file(path, "$", "fly config present"),
            FLY_ID,
        )];

        // `app = "name"` gives the conventional hostname. Likely, not Certain:
        // the app may not be deployed, and a custom domain may front it.
        let doc: Result<toml_edit::DocumentMut, _> = text.parse();
        if let Ok(doc) = doc {
            if let Some(app) = doc.get("app").and_then(|v| v.as_str()) {
                out.push(text_finding(
                    FieldPath::DeployUrl,
                    format!("https://{app}.fly.dev"),
                    Confidence::Likely,
                    Specificity::Config,
                    Evidence::from_file(path, "app", app),
                    FLY_ID,
                ));
            }
        }
        Ok(out)
    }
}

/// `.env.example` — key names only, never values.
#[derive(Debug)]
pub struct EnvExample;

const ENV_ID: DetectorId = DetectorId("deploy.env_example");

impl Detector for EnvExample {
    fn id(&self) -> DetectorId {
        ENV_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[
            Interest::FileName(".env.example"),
            Interest::FileName(".env.sample"),
            Interest::FileName(".env.template"),
        ]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::DeployEnvRequired]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let mut out = Vec::new();
        for rel in [".env.example", ".env.sample", ".env.template"] {
            if !ctx.files.has_file(rel) {
                continue;
            }
            let Ok(text) = ctx.read_text(rel) else {
                continue;
            };
            let path = std::path::Path::new(rel);

            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, _value)) = line.split_once('=') else {
                    continue;
                };
                let key = key.trim().trim_start_matches("export ").trim();
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                out.push(text_finding(
                    FieldPath::DeployEnvRequired,
                    key,
                    Confidence::Certain,
                    Specificity::Config,
                    // The excerpt is the key, never the value. A committed
                    // `.env.example` sometimes holds a real credential by
                    // mistake, and this must not copy it into the manifest or
                    // the cache.
                    Evidence::from_file(path, "line", key),
                    ENV_ID,
                ));
            }
        }
        Ok(out)
    }
}

/// A minimal string-field lookup, to avoid deriving a struct per config file.
fn json_string_field(text: &str, field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value.get(field)?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_field_lookup_tolerates_rubbish() {
        assert_eq!(
            json_string_field(r#"{"name":"macroring"}"#, "name"),
            Some("macroring".to_owned())
        );
        assert_eq!(json_string_field("not json", "name"), None);
        assert_eq!(json_string_field(r#"{"name":42}"#, "name"), None);
        assert_eq!(json_string_field("{}", "name"), None);
    }
}
