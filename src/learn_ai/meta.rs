//! Struct- and frontmatter-level validation (Phase 1, Block C).
//!
//! `crawl.rs` walks the content tree and classifies each file; this module reads each
//! classified file's contents and validates its structure, keeping `crawl.rs` focused on
//! the walk. Block C lands incrementally:
//!
//! - **Task 1:** establish the module, the per-file dispatch entry point
//!   ([`validate_file`]), and the read step ([`read_content`]) — surfacing IO/read failures
//!   as `error`-severity [`Diagnostic`]s rather than panicking, so one unreadable file never
//!   aborts the whole run.
//! - **Task 2:** deserialize `FileKind::LearnModuleJson` into a strict
//!   [`ModuleFile`] / [`ModuleMeta`] and emit a precise-locator [`Diagnostic`] for every missing
//!   required field, bad enum value, or malformed format.
//! - **Task 3 (this commit):** deserialize `FileKind::PathMetadataJson` into [`PathMeta`] and
//!   emit a precise-locator [`Diagnostic`] for every missing required field or bad `level` enum
//!   value. `level` is validated case-insensitively so live capitalised values are accepted.
//! - **Task 4 (this commit):** add real-YAML MDX frontmatter parsing — extract the leading
//!   `---\n…\n---` block and parse it with `serde_yaml`; require `title`, `description`,
//!   `duration`, `difficulty`, `lastUpdated`; validate `difficulty` enum and `duration` format.
//! - **Task 5:** wire [`validate_file`] into `validate()` so the diagnostics reach the `Report`.

use serde::Deserialize;

use super::crawl::{ContentFile, FileKind};
use crate::Diagnostic;
use crate::shared::{extract_frontmatter, is_kebab_case, non_empty};

/// Read a classified content file's contents from disk.
///
/// On any read failure (missing file, permission error, non-UTF-8 bytes) this returns a single
/// `error`-severity [`Diagnostic`] located at the file's relative path rather than panicking or
/// propagating an `Err` — so one unreadable file is reported and the run continues.
pub(crate) fn read_content(cf: &ContentFile) -> Result<String, Diagnostic> {
    std::fs::read_to_string(&cf.path)
        .map_err(|e| Diagnostic::error(cf.rel.clone(), "", format!("could not read file: {e}")))
}

/// Validate a single classified content file, dispatching on its [`FileKind`].
///
/// Returns every diagnostic the file produces (an empty vector means the file is clean).
///
/// Read failures (Task 1) short-circuit to a single `error` diagnostic. Otherwise the file is
/// dispatched by kind: Task 2 validates [`FileKind::LearnModuleJson`]; path `metadata.json`
/// (Task 3) and MDX frontmatter (Task 4) layer in their checks here later.
pub fn validate_file(cf: &ContentFile) -> Vec<Diagnostic> {
    let contents = match read_content(cf) {
        Ok(contents) => contents,
        Err(diag) => return vec![diag],
    };

    match cf.kind {
        FileKind::LearnModuleJson => validate_module_json(cf, &contents),
        FileKind::PathMetadataJson => validate_path_metadata_json(cf, &contents),
        FileKind::ModuleMdx => validate_module_mdx(cf, &contents),
    }
}

// ---------------------------------------------------------------------------
// Module `.json` structs (`FileKind::LearnModuleJson`)
// ---------------------------------------------------------------------------

/// A learn-module `.json` file: a `metadata` object plus a `sections[]` array.
///
/// Both fields are `Option` so that a *missing* `metadata` or `sections` key is reported as a
/// precise-locator diagnostic rather than aborting deserialization. Unknown top-level keys are
/// tolerated (the live files carry extras), so `deny_unknown_fields` is deliberately not used.
#[derive(Debug, Deserialize)]
pub struct ModuleFile {
    pub metadata: Option<ModuleMeta>,
    pub sections: Option<Vec<ModuleSection>>,
}

/// The `metadata` block of a learn-module `.json`.
///
/// Every required field is modelled as `Option` so presence is checked field-by-field and each
/// absence yields its own diagnostic (e.g. `metadata.duration`). Extra live keys (`author`,
/// `estimatedCompletionTime`, `prerequisites`, …) are ignored.
#[derive(Debug, Deserialize)]
pub struct ModuleMeta {
    pub id: Option<String>,
    #[serde(rename = "pathId")]
    pub path_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub duration: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub difficulty: Option<String>,
    pub order: Option<i64>,
    pub objectives: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub version: Option<String>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// One entry of a learn-module's `sections[]`. Required: `id`, `type`, `order`. Extra keys
/// (`title`, `estimatedDuration`, `content`, …) are tolerated.
#[derive(Debug, Deserialize)]
pub struct ModuleSection {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub order: Option<i64>,
}

/// Validate a learn-module `.json`: strict required-field, enum, and format checks.
///
/// A JSON parse failure yields a single whole-file `error` diagnostic; otherwise every missing
/// required field, bad enum value, and malformed format becomes its own precise-locator
/// diagnostic. Never panics, never aborts the run.
pub(crate) fn validate_module_json(cf: &ContentFile, contents: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let module: ModuleFile = match serde_json::from_str(contents) {
        Ok(m) => m,
        Err(e) => {
            return vec![Diagnostic::error(
                cf.rel.clone(),
                "",
                format!("invalid module JSON: {e}"),
            )];
        }
    };

    match &module.metadata {
        None => diags.push(missing(cf, "metadata")),
        Some(meta) => validate_module_metadata(cf, meta, &mut diags),
    }

    match &module.sections {
        None => diags.push(missing(cf, "sections")),
        Some(sections) if sections.is_empty() => diags.push(Diagnostic::error(
            cf.rel.clone(),
            "sections",
            "sections[] must contain at least one section",
        )),
        Some(sections) => {
            for (i, section) in sections.iter().enumerate() {
                validate_module_section(cf, i, section, &mut diags);
            }
        }
    }

    diags
}

/// Validate the `metadata` block: required fields, then `id`/`duration` formats and
/// `difficulty`/`type` enums (each only when the field is present).
fn validate_module_metadata(cf: &ContentFile, meta: &ModuleMeta, diags: &mut Vec<Diagnostic>) {
    require_str(cf, "metadata.id", &meta.id, diags);
    require_str(cf, "metadata.pathId", &meta.path_id, diags);
    require_str(cf, "metadata.title", &meta.title, diags);
    require_str(cf, "metadata.description", &meta.description, diags);
    require_str(cf, "metadata.duration", &meta.duration, diags);
    require_str(cf, "metadata.type", &meta.type_, diags);
    require_str(cf, "metadata.difficulty", &meta.difficulty, diags);
    if meta.order.is_none() {
        diags.push(missing(cf, "metadata.order"));
    }
    if meta.objectives.is_none() {
        diags.push(missing(cf, "metadata.objectives"));
    }
    if meta.tags.is_none() {
        diags.push(missing(cf, "metadata.tags"));
    }
    require_str(cf, "metadata.version", &meta.version, diags);
    require_str(cf, "metadata.lastUpdated", &meta.last_updated, diags);

    if let Some(id) = non_empty(&meta.id)
        && !is_kebab_case(id)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "metadata.id",
            format!("id must be kebab-case (^[a-z0-9]+(-[a-z0-9]+)*$): {id}"),
        ));
    }
    if let Some(duration) = non_empty(&meta.duration)
        && !is_valid_duration(duration)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "metadata.duration",
            format!("duration must match ^\\d+\\s+(minutes?|hours?)$: {duration}"),
        ));
    }
    if let Some(difficulty) = non_empty(&meta.difficulty)
        && !is_valid_difficulty(difficulty)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "metadata.difficulty",
            format!("difficulty must be beginner|intermediate|advanced: {difficulty}"),
        ));
    }
    if let Some(type_) = non_empty(&meta.type_)
        && !is_valid_module_type(type_)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "metadata.type",
            format!("type must be theory|concept|practice|project|assessment: {type_}"),
        ));
    }
}

/// Validate one section: required `id`/`type`/`order`, then the section `type` enum.
fn validate_module_section(
    cf: &ContentFile,
    index: usize,
    section: &ModuleSection,
    diags: &mut Vec<Diagnostic>,
) {
    require_str(cf, &format!("sections[{index}].id"), &section.id, diags);
    require_str(
        cf,
        &format!("sections[{index}].type"),
        &section.type_,
        diags,
    );
    if section.order.is_none() {
        diags.push(missing(cf, &format!("sections[{index}].order")));
    }

    if let Some(type_) = non_empty(&section.type_)
        && !is_valid_section_type(type_)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            format!("sections[{index}].type"),
            format!("type must be content|quiz|exercise|project|assessment: {type_}"),
        ));
    }
}

// ---------------------------------------------------------------------------
// Path `metadata.json` structs (`FileKind::PathMetadataJson`)
// ---------------------------------------------------------------------------

/// A path-level `metadata.json` file.
///
/// Every required field is modelled as `Option` so each absence yields its own diagnostic.
/// Extra live keys (`difficulty`, `totalDuration`, `author`, `prerequisites`, `outcomes`,
/// `resources`) are tolerated — `deny_unknown_fields` is deliberately not used.
#[derive(Debug, Deserialize)]
pub struct PathMeta {
    pub id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub level: Option<String>,
    pub duration: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
    pub topics: Option<Vec<serde_json::Value>>,
    pub modules: Option<Vec<serde_json::Value>>,
}

/// Validate a path `metadata.json`: strict required-field and `level`/`duration` format checks.
///
/// A JSON parse failure yields a single whole-file `error` diagnostic; otherwise every missing
/// required field and bad `level` enum value becomes its own precise-locator diagnostic.
/// Never panics, never aborts the run.
///
/// `level` is validated case-insensitively: live files use capitalised values
/// (`"Intermediate"`, `"Beginner"`, `"Advanced"`), so the check lowercases the value before
/// comparing it to the allowed set — consistent with the TS validator's behaviour.
pub(crate) fn validate_path_metadata_json(cf: &ContentFile, contents: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let meta: PathMeta = match serde_json::from_str(contents) {
        Ok(m) => m,
        Err(e) => {
            return vec![Diagnostic::error(
                cf.rel.clone(),
                "",
                format!("invalid path metadata JSON: {e}"),
            )];
        }
    };

    require_str(cf, "id", &meta.id, &mut diags);
    require_str(cf, "title", &meta.title, &mut diags);
    require_str(cf, "description", &meta.description, &mut diags);
    require_str(cf, "level", &meta.level, &mut diags);
    require_str(cf, "duration", &meta.duration, &mut diags);
    require_str(cf, "version", &meta.version, &mut diags);
    require_str(cf, "lastUpdated", &meta.last_updated, &mut diags);
    if meta.topics.is_none() {
        diags.push(missing(cf, "topics"));
    }
    if meta.modules.is_none() {
        diags.push(missing(cf, "modules"));
    }

    // Validate `level` enum (case-insensitive).
    if let Some(level) = non_empty(&meta.level)
        && !is_valid_level(level)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "level",
            format!("level must be beginner|intermediate|advanced (case-insensitive): {level}"),
        ));
    }

    // Validate `duration` format.
    if let Some(duration) = non_empty(&meta.duration)
        && !is_valid_duration(duration)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "duration",
            format!("duration must match ^\\d+\\s+(minutes?|hours?)$: {duration}"),
        ));
    }

    diags
}

/// `true` if `s` is a valid path-level `level` value (case-insensitive).
///
/// Live `metadata.json` files use capitalised forms (`"Intermediate"`, `"Beginner"`,
/// `"Advanced"`), so the check lowercases before comparing.
fn is_valid_level(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "beginner" | "intermediate" | "advanced"
    )
}

// ---------------------------------------------------------------------------
// MDX frontmatter structs (`FileKind::ModuleMdx`)
// ---------------------------------------------------------------------------

/// The YAML frontmatter of a `.mdx` module file.
///
/// Every required field is modelled as `Option` so each absence yields its own diagnostic
/// (`frontmatter.<field>`). Extra keys the live files carry are tolerated — `deny_unknown_fields`
/// is deliberately not used.
#[derive(Debug, Deserialize)]
pub struct MdxFrontmatter {
    pub title: Option<String>,
    pub description: Option<String>,
    pub duration: Option<String>,
    pub difficulty: Option<String>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// Validate a `.mdx` module file: extract the leading YAML frontmatter block, parse it with
/// `serde_yaml`, and check required fields, `difficulty` enum, and `duration` format.
///
/// - No frontmatter block (missing `---` fence or unterminated block) → one `error` diagnostic.
/// - Malformed YAML inside the block → one `error` diagnostic.
/// - Each missing required field → one `error` diagnostic with locator `frontmatter.<field>`.
/// - Bad `difficulty` enum or `duration` format → one `error` diagnostic per violation.
///
/// Never panics, never aborts the run.
pub(crate) fn validate_module_mdx(cf: &ContentFile, contents: &str) -> Vec<Diagnostic> {
    let yaml = match extract_frontmatter(contents) {
        Some(y) => y,
        None => {
            return vec![Diagnostic::error(
                cf.rel.clone(),
                "frontmatter",
                "missing or unterminated frontmatter block (expected leading --- fence)",
            )];
        }
    };

    let fm: MdxFrontmatter = match serde_yaml::from_str(yaml) {
        Ok(f) => f,
        Err(e) => {
            return vec![Diagnostic::error(
                cf.rel.clone(),
                "frontmatter",
                format!("malformed YAML in frontmatter: {e}"),
            )];
        }
    };

    let mut diags = Vec::new();

    require_str(cf, "frontmatter.title", &fm.title, &mut diags);
    require_str(cf, "frontmatter.description", &fm.description, &mut diags);
    require_str(cf, "frontmatter.duration", &fm.duration, &mut diags);
    require_str(cf, "frontmatter.difficulty", &fm.difficulty, &mut diags);
    require_str(cf, "frontmatter.lastUpdated", &fm.last_updated, &mut diags);

    if let Some(difficulty) = non_empty(&fm.difficulty)
        && !is_valid_difficulty(difficulty)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "frontmatter.difficulty",
            format!("difficulty must be beginner|intermediate|advanced: {difficulty}"),
        ));
    }

    if let Some(duration) = non_empty(&fm.duration)
        && !is_valid_duration(duration)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "frontmatter.duration",
            format!("duration must match ^\\d+\\s+(minutes?|hours?)$: {duration}"),
        ));
    }

    diags
}

// ---------------------------------------------------------------------------
// Shared helpers (presence, enum, and format checks)
// ---------------------------------------------------------------------------

/// Build a "missing required field" `error` diagnostic at `locator`.
fn missing(cf: &ContentFile, locator: &str) -> Diagnostic {
    Diagnostic::error(cf.rel.clone(), locator, "missing required field")
}

/// Push a `missing` diagnostic when an optional string field is absent or empty/whitespace.
fn require_str(
    cf: &ContentFile,
    locator: &str,
    value: &Option<String>,
    diags: &mut Vec<Diagnostic>,
) {
    if non_empty(value).is_none() {
        diags.push(missing(cf, locator));
    }
}

/// `true` if `s` matches `^\d+\s+(minutes?|hours?)$`, without the `regex` crate.
fn is_valid_duration(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    let digits_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return false; // need 1+ digits
    }

    let ws_start = i;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i == ws_start {
        return false; // need 1+ whitespace
    }

    let unit: String = chars[i..].iter().collect();
    matches!(unit.as_str(), "minute" | "minutes" | "hour" | "hours")
}

/// `true` if `s` is a valid module `difficulty`.
fn is_valid_difficulty(s: &str) -> bool {
    matches!(s, "beginner" | "intermediate" | "advanced")
}

/// `true` if `s` is a valid module `type`.
fn is_valid_module_type(s: &str) -> bool {
    matches!(
        s,
        "theory" | "concept" | "practice" | "project" | "assessment"
    )
}

/// `true` if `s` is a valid section `type`.
fn is_valid_section_type(s: &str) -> bool {
    matches!(
        s,
        "content" | "quiz" | "exercise" | "project" | "assessment"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::learn_ai::crawl::{FileKind, Locale};
    use std::path::PathBuf;

    fn temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mev-meta-{suffix}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content_file(path: PathBuf, rel: &str, kind: FileKind) -> ContentFile {
        ContentFile {
            path,
            rel: PathBuf::from(rel),
            kind,
            path_id: "demo".to_string(),
            module_id: None,
            locale: Locale::En,
        }
    }

    #[test]
    fn read_content_ok_returns_file_body() {
        let dir = temp_dir("read-ok");
        let path = dir.join("metadata.json");
        std::fs::write(&path, b"{\"id\":\"demo\"}").unwrap();

        let cf = content_file(path, "paths/demo/metadata.json", FileKind::PathMetadataJson);
        let body = read_content(&cf).expect("expected a readable file");
        assert_eq!(body, "{\"id\":\"demo\"}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_content_missing_file_yields_error_diagnostic() {
        let cf = content_file(
            PathBuf::from("/nonexistent/mev/does-not-exist.json"),
            "paths/demo/metadata.json",
            FileKind::PathMetadataJson,
        );

        let diag = read_content(&cf).expect_err("expected a read failure");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.file, PathBuf::from("paths/demo/metadata.json"));
        assert_eq!(diag.locator, "");
        assert!(
            diag.message.starts_with("could not read file:"),
            "unexpected message: {}",
            diag.message
        );
    }

    #[test]
    fn validate_file_surfaces_read_failure_as_single_error() {
        let cf = content_file(
            PathBuf::from("/nonexistent/mev/missing.mdx"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );

        let diags = validate_file(&cf);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one read-failure diagnostic"
        );
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(
            diags[0].file,
            PathBuf::from("paths/demo/modules/01-intro.mdx")
        );
    }

    #[test]
    fn validate_file_dispatches_mdx_to_frontmatter_validator() {
        // A readable .mdx with a valid frontmatter block is dispatched to validate_module_mdx.
        // Task 4 wires the real validator; this file has only `title` so it must produce errors.
        let dir = temp_dir("readable-partial");
        let path = dir.join("01-intro.mdx");
        std::fs::write(&path, b"---\ntitle: x\n---\nbody").unwrap();

        let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);
        // Should produce errors for the missing required fields.
        let diags = validate_file(&cf);
        assert!(
            !diags.is_empty(),
            "expected diagnostics for incomplete frontmatter"
        );
        assert!(
            diags.iter().all(|d| d.severity == Severity::Error),
            "all diagnostics should be errors"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- module `.json` validation (Task 2) ---

    /// A fully-valid learn-module `.json` body (all required fields, valid enums/formats),
    /// carrying extra keys the live files use to confirm they are tolerated.
    fn good_module_json() -> String {
        r#"{
          "metadata": {
            "id": "intro-to-mcp",
            "pathId": "mcp-fundamentals",
            "title": "What is MCP?",
            "description": "An introduction to MCP.",
            "duration": "30 minutes",
            "type": "concept",
            "difficulty": "beginner",
            "order": 1,
            "objectives": ["Understand MCP"],
            "tags": ["mcp"],
            "version": "1.0.0",
            "lastUpdated": "2025-06-20",
            "author": "extra key tolerated",
            "estimatedCompletionTime": 30,
            "prerequisites": []
          },
          "sections": [
            { "id": "overview", "type": "content", "order": 1, "title": "Overview", "content": "..." }
          ]
        }"#
        .to_string()
    }

    fn module_cf() -> ContentFile {
        content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.json",
            FileKind::LearnModuleJson,
        )
    }

    fn locators(diags: &[Diagnostic]) -> Vec<String> {
        diags.iter().map(|d| d.locator.clone()).collect()
    }

    #[test]
    fn good_module_json_is_clean() {
        let diags = validate_module_json(&module_cf(), &good_module_json());
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn missing_duration_emits_locator() {
        let body = good_module_json().replace("\"duration\": \"30 minutes\",\n", "");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata.duration");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "missing required field");
    }

    #[test]
    fn bad_difficulty_enum_emits_locator() {
        let body = good_module_json().replace("\"beginner\"", "\"expert\"");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata.difficulty");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn non_kebab_id_emits_locator() {
        let body = good_module_json().replace("\"intro-to-mcp\"", "\"Intro_To_MCP\"");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata.id");
    }

    #[test]
    fn malformed_duration_emits_locator() {
        let body = good_module_json().replace("\"30 minutes\"", "\"half an hour\"");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata.duration");
    }

    #[test]
    fn empty_sections_emits_locator() {
        let body = good_module_json().replace(
            "\"sections\": [\n            { \"id\": \"overview\", \"type\": \"content\", \"order\": 1, \"title\": \"Overview\", \"content\": \"...\" }\n          ]",
            "\"sections\": []",
        );
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "sections");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn section_missing_id_emits_indexed_locator() {
        let body = good_module_json().replace("\"id\": \"overview\", ", "");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "sections[0].id");
    }

    #[test]
    fn bad_section_type_emits_indexed_locator() {
        let body = good_module_json().replace("\"type\": \"content\"", "\"type\": \"video\"");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "sections[0].type");
    }

    #[test]
    fn bad_module_type_emits_locator() {
        let body = good_module_json().replace("\"type\": \"concept\"", "\"type\": \"lecture\"");
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata.type");
    }

    #[test]
    fn missing_metadata_block_emits_single_locator() {
        let body = r#"{ "sections": [ { "id": "a", "type": "content", "order": 1 } ] }"#;
        let diags = validate_module_json(&module_cf(), body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "metadata");
    }

    #[test]
    fn missing_sections_block_emits_locator() {
        let body = good_module_json().replace(
            ",\n          \"sections\": [\n            { \"id\": \"overview\", \"type\": \"content\", \"order\": 1, \"title\": \"Overview\", \"content\": \"...\" }\n          ]",
            "",
        );
        let diags = validate_module_json(&module_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "sections");
    }

    #[test]
    fn invalid_json_emits_single_whole_file_error() {
        let diags = validate_module_json(&module_cf(), "{ not valid json ");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "");
        assert!(
            diags[0].message.starts_with("invalid module JSON:"),
            "unexpected message: {}",
            diags[0].message
        );
    }

    #[test]
    fn all_required_metadata_fields_reported_when_metadata_empty() {
        let body =
            r#"{ "metadata": {}, "sections": [ { "id": "a", "type": "content", "order": 1 } ] }"#;
        let diags = validate_module_json(&module_cf(), body);
        let got = locators(&diags);
        for expected in [
            "metadata.id",
            "metadata.pathId",
            "metadata.title",
            "metadata.description",
            "metadata.duration",
            "metadata.type",
            "metadata.difficulty",
            "metadata.order",
            "metadata.objectives",
            "metadata.tags",
            "metadata.version",
            "metadata.lastUpdated",
        ] {
            assert!(
                got.contains(&expected.to_string()),
                "missing locator {expected} in {got:?}"
            );
        }
    }

    // --- path `metadata.json` validation (Task 3) ---

    /// A fully-valid path `metadata.json` body (all required fields, valid values), carrying
    /// extra keys the live files use to confirm they are tolerated.
    fn good_path_metadata_json() -> String {
        r#"{
          "id": "mcp-fundamentals",
          "title": "MCP Fundamentals",
          "description": "A learning path covering MCP from the ground up.",
          "level": "Intermediate",
          "duration": "4 hours",
          "version": "1.0.0",
          "lastUpdated": "2025-06-20",
          "topics": ["mcp", "agents"],
          "modules": ["intro-to-mcp", "advanced-mcp"],
          "author": "Brandon J. Redmond",
          "totalDuration": 240,
          "prerequisites": [],
          "outcomes": ["Understand MCP"],
          "resources": []
        }"#
        .to_string()
    }

    fn path_meta_cf() -> ContentFile {
        content_file(
            PathBuf::from("/unused"),
            "paths/mcp-fundamentals/metadata.json",
            FileKind::PathMetadataJson,
        )
    }

    #[test]
    fn good_path_metadata_is_clean() {
        let diags = validate_path_metadata_json(&path_meta_cf(), &good_path_metadata_json());
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn path_metadata_level_case_insensitive_lowercase() {
        let body = good_path_metadata_json().replace("\"Intermediate\"", "\"intermediate\"");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert!(
            diags.is_empty(),
            "lowercase level should be valid, got: {diags:?}"
        );
    }

    #[test]
    fn path_metadata_level_case_insensitive_capitalized() {
        let body = good_path_metadata_json().replace("\"Intermediate\"", "\"Beginner\"");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert!(
            diags.is_empty(),
            "capitalized level should be valid, got: {diags:?}"
        );
    }

    #[test]
    fn path_metadata_bad_level_emits_locator() {
        let body = good_path_metadata_json().replace("\"Intermediate\"", "\"Expert\"");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "level");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn path_metadata_missing_level_emits_locator() {
        let body = good_path_metadata_json().replace("\"level\": \"Intermediate\",\n", "");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "level");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "missing required field");
    }

    #[test]
    fn path_metadata_missing_modules_emits_locator() {
        let body = good_path_metadata_json()
            .replace("\"modules\": [\"intro-to-mcp\", \"advanced-mcp\"],\n", "");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "modules");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "missing required field");
    }

    #[test]
    fn path_metadata_missing_topics_emits_locator() {
        let body = good_path_metadata_json().replace("\"topics\": [\"mcp\", \"agents\"],\n", "");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "topics");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn path_metadata_malformed_duration_emits_locator() {
        let body = good_path_metadata_json().replace("\"4 hours\"", "\"a few hours\"");
        let diags = validate_path_metadata_json(&path_meta_cf(), &body);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "duration");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn path_metadata_invalid_json_emits_whole_file_error() {
        let diags = validate_path_metadata_json(&path_meta_cf(), "{ not valid json ");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "");
        assert!(
            diags[0].message.starts_with("invalid path metadata JSON:"),
            "unexpected message: {}",
            diags[0].message
        );
    }

    #[test]
    fn path_metadata_all_required_fields_reported_when_empty() {
        let body = r#"{}"#;
        let diags = validate_path_metadata_json(&path_meta_cf(), body);
        let got = locators(&diags);
        for expected in [
            "id",
            "title",
            "description",
            "level",
            "duration",
            "version",
            "lastUpdated",
            "topics",
            "modules",
        ] {
            assert!(
                got.contains(&expected.to_string()),
                "missing locator {expected} in {got:?}"
            );
        }
    }

    #[test]
    fn validate_file_dispatches_path_metadata_json() {
        let dir = temp_dir("dispatch-path-meta");
        let path = dir.join("metadata.json");
        std::fs::write(&path, good_path_metadata_json().as_bytes()).unwrap();

        let cf = content_file(
            path,
            "paths/mcp-fundamentals/metadata.json",
            FileKind::PathMetadataJson,
        );
        let diags = validate_file(&cf);
        assert!(
            diags.is_empty(),
            "good path metadata should have no diagnostics, got: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- helper unit tests ---

    #[test]
    fn duration_helper() {
        assert!(is_valid_duration("30 minutes"));
        assert!(is_valid_duration("1 minute"));
        assert!(is_valid_duration("2 hours"));
        assert!(is_valid_duration("1 hour"));
        assert!(!is_valid_duration("30minutes"));
        assert!(!is_valid_duration("half an hour"));
        assert!(!is_valid_duration("30 mins"));
        assert!(!is_valid_duration("minutes"));
        assert!(!is_valid_duration(""));
    }

    #[test]
    fn enum_helpers() {
        assert!(is_valid_difficulty("beginner"));
        assert!(!is_valid_difficulty("expert"));
        assert!(is_valid_module_type("concept"));
        assert!(!is_valid_module_type("lecture"));
        assert!(is_valid_section_type("quiz"));
        assert!(!is_valid_section_type("video"));
    }

    #[test]
    fn level_helper_case_insensitive() {
        assert!(is_valid_level("beginner"));
        assert!(is_valid_level("intermediate"));
        assert!(is_valid_level("advanced"));
        // Capitalised live values must be accepted.
        assert!(is_valid_level("Beginner"));
        assert!(is_valid_level("Intermediate"));
        assert!(is_valid_level("Advanced"));
        // Invalid values.
        assert!(!is_valid_level("expert"));
        assert!(!is_valid_level("Expert"));
        assert!(!is_valid_level(""));
    }

    // --- MDX frontmatter validation (Task 4) ---

    /// A fully-valid `.mdx` frontmatter block (all required fields, valid enums/formats).
    fn good_mdx_body() -> &'static str {
        "---\ntitle: Introduction to MCP\ndescription: A short intro.\nduration: 30 minutes\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\nextra_key: tolerated\n---\n\n# Body text\n"
    }

    fn mdx_cf(path: std::path::PathBuf) -> ContentFile {
        content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx)
    }

    #[test]
    fn good_mdx_frontmatter_is_clean() {
        let dir = temp_dir("mdx-good");
        let path = dir.join("01-intro.mdx");
        std::fs::write(&path, good_mdx_body()).unwrap();

        let cf = mdx_cf(path);
        let diags = validate_file(&cf);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mdx_no_frontmatter_emits_single_error() {
        let dir = temp_dir("mdx-no-fm");
        let path = dir.join("01-intro.mdx");
        std::fs::write(&path, b"# Just body, no frontmatter\n").unwrap();

        let cf = mdx_cf(path);
        let diags = validate_module_mdx(&cf, "# Just body, no frontmatter\n");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "frontmatter");
        assert!(
            diags[0].message.contains("missing or unterminated"),
            "unexpected message: {}",
            diags[0].message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mdx_unterminated_frontmatter_emits_single_error() {
        let contents = "---\ntitle: Foo\ndescription: Bar\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "frontmatter");
    }

    #[test]
    fn mdx_malformed_yaml_emits_single_error() {
        // Intentionally invalid YAML inside the frontmatter fences.
        let contents = "---\ntitle: [unclosed bracket\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "frontmatter");
        assert!(
            diags[0].message.contains("malformed YAML"),
            "unexpected message: {}",
            diags[0].message
        );
    }

    #[test]
    fn mdx_missing_duration_emits_locator() {
        let contents = "---\ntitle: T\ndescription: D\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "frontmatter.duration");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "missing required field");
    }

    #[test]
    fn mdx_missing_description_emits_locator() {
        let contents = "---\ntitle: T\nduration: 30 minutes\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "frontmatter.description");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn mdx_bad_difficulty_enum_emits_locator() {
        let contents = "---\ntitle: T\ndescription: D\nduration: 30 minutes\ndifficulty: expert\nlastUpdated: \"2025-06-20\"\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "frontmatter.difficulty");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn mdx_malformed_duration_emits_locator() {
        let contents = "---\ntitle: T\ndescription: D\nduration: half an hour\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "frontmatter.duration");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn mdx_missing_last_updated_emits_locator() {
        let contents = "---\ntitle: T\ndescription: D\nduration: 30 minutes\ndifficulty: beginner\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].locator, "frontmatter.lastUpdated");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn mdx_all_required_fields_missing_reports_each() {
        let contents = "---\nextra: only\n---\nbody\n";
        let cf = content_file(
            PathBuf::from("/unused"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );
        let diags = validate_module_mdx(&cf, contents);
        let locs: Vec<_> = diags.iter().map(|d| d.locator.as_str()).collect();
        for expected in [
            "frontmatter.title",
            "frontmatter.description",
            "frontmatter.duration",
            "frontmatter.difficulty",
            "frontmatter.lastUpdated",
        ] {
            assert!(
                locs.contains(&expected),
                "missing locator {expected} in {locs:?}"
            );
        }
    }
}
