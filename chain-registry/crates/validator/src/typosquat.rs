// crates/validator/src/typosquat.rs
// Levenshtein-distance based typosquatting detector.
// Checks if a package name is suspiciously close to a popular package name
// and flags it as a potential typosquat attack.

use once_cell::sync::Lazy;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TyposquatDataset {
    version: u32,
    packages: HashMap<String, Vec<String>>,
}

/// Compiled-in typosquat dataset (loaded from data/typosquat.json at build time).
static DATASET: Lazy<TyposquatDataset> = Lazy::new(|| {
    let json = include_str!("../data/typosquat.json");
    serde_json::from_str(json).expect("typosquat.json must be valid JSON")
});

/// Result of a typosquat check.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TyposquatMatch {
    pub candidate:  String,   // the package being checked
    pub target:     String,   // the popular package it resembles
    pub distance:   usize,    // edit distance
    pub ecosystem:  String,
}

fn normalise(name: &str) -> String {
    name.to_lowercase()
        .trim_start_matches('@')
        .split('/')
        .last()
        .unwrap_or(name)
        .replace(['-', '_', '.'], "")
}

/// Check whether `name` in `ecosystem` looks like a typosquat of a popular package.
/// Returns Some(match) if a suspiciously close name is found.
pub fn check(name: &str, ecosystem: &str) -> Option<TyposquatMatch> {
    let candidates = DATASET.packages.get(ecosystem)?;
    let normalised = normalise(name);

    for popular in candidates {
        let pop_norm = normalise(popular);
        // Skip exact matches — those are the real packages.
        if normalised == pop_norm {
            return None;
        }

        let dist = strsim::levenshtein(&normalised, &pop_norm);

        // Flag if within edit distance threshold.
        let min_len = normalised.len().min(pop_norm.len());
        let threshold = if min_len < 5 { 0 } else if min_len < 8 { 1 } else { 2 };

        if dist > 0 && dist <= threshold {
            return Some(TyposquatMatch {
                candidate: name.to_string(),
                target:    popular.clone(),
                distance:  dist,
                ecosystem: ecosystem.to_string(),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_not_flagged() {
        assert!(check("express", "npm").is_none());
        assert!(check("requests", "pypi").is_none());
    }

    #[test]
    fn obvious_typosquat_flagged() {
        // "expres" is edit distance 1 from "express"
        let m = check("expres", "npm");
        assert!(m.is_some());
        assert_eq!(m.unwrap().target, "express");
    }

    #[test]
    fn scoped_package_checked_correctly() {
        // "@scope/expres" should still be caught
        let m = check("@scope/expres", "npm");
        assert!(m.is_some());
    }

    #[test]
    fn unrelated_package_not_flagged() {
        assert!(check("my-totally-unique-lib-xyz", "npm").is_none());
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(strsim::levenshtein("kitten", "sitting"), 3);
        assert_eq!(strsim::levenshtein("", "abc"), 3);
        assert_eq!(strsim::levenshtein("abc", "abc"), 0);
        assert_eq!(strsim::levenshtein("abc", "ab"), 1);
    }
}
