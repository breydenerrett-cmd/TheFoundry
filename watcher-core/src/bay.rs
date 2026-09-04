//! Phase 4E — six-bay project resolution (§9). Repo-first mapping; falls
//! back to EXPERIMENTS (known-but-unmatched) or PERSONAL/MISC (no repo hint
//! at all) rather than a silent catch-all. Room-level detail inside
//! SPORTS LAB / AI BUSINESS COMPLEX is a rendering grouping only, per §4 —
//! not implemented as a separate layer yet; bay-level is what Phase 4 needs.

pub const BAYS: &[&str] = &["SPORTS LAB", "AI BUSINESS COMPLEX", "SERVERFORGE", "MUSIC LAB", "EXPERIMENTS", "PERSONAL/MISC"];

/// `repo_hint` is whatever a source observer could tell us about origin —
/// a repo path, a cwd, a URL fragment. Substring match, case-insensitive.
pub fn resolve_bay(repo_hint: Option<&str>) -> &'static str {
    let Some(hint) = repo_hint else { return "PERSONAL/MISC" };
    let h = hint.to_lowercase();
    if h.contains("aisportsanalysis") || h.contains("sports") || h.contains("linehound") {
        "SPORTS LAB"
    } else if h.contains("resume-business") || h.contains("agency") || h.contains("pureline") || h.contains("lead-gen") || h.contains("lead_gen") {
        "AI BUSINESS COMPLEX"
    } else if h.contains("fivem") || h.contains("qbcore") || h.contains("serverforge") {
        "SERVERFORGE"
    } else if h.contains("music") {
        "MUSIC LAB"
    } else {
        "EXPERIMENTS"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_repos_map_to_their_bay() {
        assert_eq!(resolve_bay(Some("/home/user/aisportsanalysis")), "SPORTS LAB");
        assert_eq!(resolve_bay(Some("github.com/breydenerrett-cmd/resume-business")), "AI BUSINESS COMPLEX");
    }

    #[test]
    fn no_hint_is_personal_misc_not_experiments() {
        assert_eq!(resolve_bay(None), "PERSONAL/MISC");
    }

    #[test]
    fn unmatched_but_present_hint_is_experiments_not_silently_dropped() {
        assert_eq!(resolve_bay(Some("/home/user/some-new-thing")), "EXPERIMENTS");
    }
}
