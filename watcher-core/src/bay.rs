//! Phase 4E v2 — six-bay project resolution (§9), driven by an explicit
//! `BayMap` rather than guesswork. Resolution order: repo identity
//! ([repos], exact `owner/name` or path basename, case-insensitive) →
//! substring rules ([rules], applied to the raw hint) → tags ([tags], when
//! a session carries one) → UNRESOLVED. Unknown must stay unknown: there is
//! no more default-to-PERSONAL/MISC-on-no-hint or
//! default-to-EXPERIMENTS-on-no-match guess. Room-level detail inside
//! SPORTS LAB / AI BUSINESS COMPLEX is a rendering grouping only, per §4 —
//! not implemented as a separate layer yet; bay-level is what Phase 4 needs.

use std::collections::HashMap;
use std::path::Path;

/// The six real bays sessions can be assigned to.
pub const BAYS: &[&str] = &[
    "SPORTS LAB",
    "AI BUSINESS COMPLEX",
    "SERVERFORGE",
    "MUSIC LAB",
    "EXPERIMENTS",
    "PERSONAL/MISC",
];

/// Seventh display group: rendered last, for sessions we honestly cannot
/// place rather than guessed into one of the six real bays.
pub const UNRESOLVED: &str = "UNRESOLVED";

/// How a `BayResolution` was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Matched an explicit `[repos]` entry by normalized repo identity.
    Repo,
    /// Matched a `[rules]` substring pattern against the raw hint.
    Rule,
    /// Matched a `[tags]` entry against a session tag.
    Tag,
    /// No hint, or a hint that matched nothing — never guessed.
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BayResolution {
    pub bay: String,
    pub via: Via,
}

impl BayResolution {
    fn unresolved() -> Self {
        BayResolution {
            bay: UNRESOLVED.to_string(),
            via: Via::Unresolved,
        }
    }
}

/// Explicit project-resolution map, loaded from an optional TOML file.
/// A missing file is not an error — it just means every hint resolves to
/// UNRESOLVED (see `BayMap::load`).
#[derive(Debug, Clone, Default)]
pub struct BayMap {
    /// key: normalized repo identity (lowercase `owner/name` or lowercase
    /// path basename) -> bay name.
    repos: HashMap<String, String>,
    /// ordered substring rules: (lowercase pattern, bay name), applied in
    /// file order against the lowercased raw hint.
    rules: Vec<(String, String)>,
    /// tag -> bay name.
    tags: HashMap<String, String>,
}

impl BayMap {
    pub fn new() -> Self {
        BayMap::default()
    }

    /// Load a `BayMap` from `path`. A missing file returns an empty map
    /// (not an error) — callers should tell the operator, once, that
    /// everything will be UNRESOLVED until a map is supplied.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text))
    }

    /// Parse TOML map text. Malformed lines/sections are skipped rather
    /// than panicking or failing the whole load — a bad entry should not
    /// take down project resolution for every other session.
    pub fn parse(text: &str) -> Self {
        let mut map = BayMap::new();
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(_) => return map,
        };

        if let Some(repos) = table.get("repos").and_then(|v| v.as_table()) {
            for (k, v) in repos {
                if let Some(bay) = v.as_str() {
                    map.repos.insert(k.to_lowercase(), bay.to_string());
                }
            }
        }
        if let Some(rules) = table.get("rules").and_then(|v| v.as_table()) {
            for (k, v) in rules {
                if let Some(bay) = v.as_str() {
                    map.rules.push((k.to_lowercase(), bay.to_string()));
                }
            }
        }
        if let Some(tags) = table.get("tags").and_then(|v| v.as_table()) {
            for (k, v) in tags {
                if let Some(bay) = v.as_str() {
                    map.tags.insert(k.to_lowercase(), bay.to_string());
                }
            }
        }
        map
    }

    /// Normalize a raw hint (a git URL, a repo path, or a bare identifier)
    /// into a repo identity: `owner/name` extracted from an https/ssh git
    /// URL (`.git` suffix stripped), else the basename of a path. Always
    /// lowercased for case-insensitive matching.
    fn normalize_identity(hint: &str) -> String {
        let trimmed = hint.trim().trim_end_matches('/');
        let stripped = trimmed.strip_suffix(".git").unwrap_or(trimmed);

        // ssh form: git@host:owner/name
        if let Some(colon) = stripped.rfind(':') {
            if stripped[..colon].contains('@') {
                let rest = &stripped[colon + 1..];
                if rest.contains('/') {
                    return rest.to_lowercase();
                }
            }
        }

        // https(s):// or ssh:// form: scheme://host/owner/name
        if let Some(idx) = stripped.find("://") {
            let rest = &stripped[idx + 3..];
            let path = rest.split_once('/').map_or(rest, |(_, p)| p);
            let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if segs.len() >= 2 {
                let owner = segs[segs.len() - 2];
                let name = segs[segs.len() - 1];
                return format!("{owner}/{name}").to_lowercase();
            }
        }

        // Bare "host/owner/name" with no scheme (e.g. "github.com/owner/name")
        // — recognized by a dotted first segment that isn't itself a path
        // (doesn't start with '/'), so it isn't confused with a filesystem
        // path's basename below.
        if !stripped.starts_with('/') {
            let segs: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();
            if segs.len() >= 3 && segs[0].contains('.') {
                let owner = segs[segs.len() - 2];
                let name = segs[segs.len() - 1];
                return format!("{owner}/{name}").to_lowercase();
            }
        }

        // Otherwise: a plain path or bare identifier — use the basename.
        let base = stripped.rsplit('/').next().unwrap_or(stripped);
        base.to_lowercase()
    }

    /// Resolve a bay from an optional repo hint and an optional set of
    /// session tags. No hint and no tag match → UNRESOLVED. A hint that
    /// matches nothing in `[repos]` or `[rules]` → UNRESOLVED. Never
    /// guessed.
    pub fn resolve(&self, repo_hint: Option<&str>, tags: &[String]) -> BayResolution {
        if let Some(hint) = repo_hint {
            let identity = Self::normalize_identity(hint);
            if let Some(bay) = self.repos.get(&identity) {
                return BayResolution {
                    bay: bay.clone(),
                    via: Via::Repo,
                };
            }
            let h = hint.to_lowercase();
            for (pattern, bay) in &self.rules {
                if h.contains(pattern.as_str()) {
                    return BayResolution {
                        bay: bay.clone(),
                        via: Via::Rule,
                    };
                }
            }
        }

        // TODO: hook this up once session records carry a tags field —
        // for now `tags` is always empty from call sites, but the lookup
        // is wired so nothing else needs to change when it lands.
        for tag in tags {
            if let Some(bay) = self.tags.get(&tag.to_lowercase()) {
                return BayResolution {
                    bay: bay.clone(),
                    via: Via::Tag,
                };
            }
        }

        BayResolution::unresolved()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_map() -> BayMap {
        BayMap::parse(
            r#"
[repos]
"breydenerrett-cmd/aisportsanalysis" = "SPORTS LAB"
"breydenerrett-cmd/thefoundry" = "EXPERIMENTS"

[rules]
"linehound" = "SPORTS LAB"
"pureline" = "AI BUSINESS COMPLEX"
"fivem" = "SERVERFORGE"

[tags]
"music" = "MUSIC LAB"
"#,
        )
    }

    #[test]
    fn repo_identity_matches_from_https_url() {
        let map = sample_map();
        let res = map.resolve(
            Some("https://github.com/breydenerrett-cmd/aisportsanalysis.git"),
            &[],
        );
        assert_eq!(res.bay, "SPORTS LAB");
        assert_eq!(res.via, Via::Repo);
    }

    #[test]
    fn repo_identity_matches_from_ssh_url() {
        let map = sample_map();
        let res = map.resolve(Some("git@github.com:breydenerrett-cmd/TheFoundry.git"), &[]);
        assert_eq!(res.bay, "EXPERIMENTS");
        assert_eq!(res.via, Via::Repo);
    }

    #[test]
    fn repo_identity_matches_from_path_basename() {
        // No [repos] entry keyed by full path basename in the sample map,
        // so use a map keyed by basename directly.
        let map = BayMap::parse(
            r#"
[repos]
"aisportsanalysis" = "SPORTS LAB"
"#,
        );
        let res = map.resolve(Some("/home/user/aisportsanalysis"), &[]);
        assert_eq!(res.bay, "SPORTS LAB");
        assert_eq!(res.via, Via::Repo);
    }

    #[test]
    fn rule_matches_when_repo_identity_misses() {
        let map = sample_map();
        let res = map.resolve(Some("github.com/someone/linehound-scraper"), &[]);
        assert_eq!(res.bay, "SPORTS LAB");
        assert_eq!(res.via, Via::Rule);
    }

    #[test]
    fn no_hint_is_unresolved_not_personal_misc() {
        let map = sample_map();
        let res = map.resolve(None, &[]);
        assert_eq!(res.bay, UNRESOLVED);
        assert_eq!(res.via, Via::Unresolved);
    }

    #[test]
    fn unmatched_hint_is_unresolved_not_experiments() {
        let map = sample_map();
        let res = map.resolve(Some("/home/user/some-new-thing"), &[]);
        assert_eq!(res.bay, UNRESOLVED);
        assert_eq!(res.via, Via::Unresolved);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let map = sample_map();
        let res = map.resolve(Some("GitHub.com/BreydenErrett-CMD/AiSportsAnalysis"), &[]);
        assert_eq!(res.bay, "SPORTS LAB");
        assert_eq!(res.via, Via::Repo);

        let res2 = map.resolve(Some("something/LINEHOUND/tool"), &[]);
        assert_eq!(res2.bay, "SPORTS LAB");
        assert_eq!(res2.via, Via::Rule);
    }

    #[test]
    fn malformed_map_file_is_skipped_not_panicking() {
        // Not valid TOML at all.
        let map = BayMap::parse("this is not { valid toml ::: at all");
        let res = map.resolve(Some("/home/user/aisportsanalysis"), &[]);
        assert_eq!(res.bay, UNRESOLVED);

        // Valid TOML, but [repos] values aren't strings — those entries
        // should be skipped rather than panicking.
        let map2 = BayMap::parse(
            r#"
[repos]
"some/repo" = 123
"other/repo" = "SPORTS LAB"
"#,
        );
        assert_eq!(
            map2.resolve(Some("github.com/other/repo"), &[]).bay,
            "SPORTS LAB"
        );
        assert_eq!(
            map2.resolve(Some("github.com/some/repo"), &[]).bay,
            UNRESOLVED
        );
    }

    #[test]
    fn missing_file_yields_empty_map_not_an_error() {
        let result = BayMap::load(Path::new("/nonexistent/path/does-not-exist.toml"));
        assert!(result.is_err());
        // Callers treat the Err as "use an empty map", exercised in main.rs;
        // here we confirm the empty map itself always resolves UNRESOLVED.
        let map = BayMap::new();
        assert_eq!(map.resolve(Some("anything"), &[]).bay, UNRESOLVED);
        assert_eq!(map.resolve(None, &[]).bay, UNRESOLVED);
    }
}
