// crates/validator/src/typosquat.rs
// Levenshtein-distance based typosquatting detector.
// Checks if a package name is suspiciously close to a popular package name
// and flags it as a potential typosquat attack.

/// Well-known popular package names that are common typosquatting targets.
/// In production this list would be loaded from a maintained database.
const POPULAR_NPM: &[&str] = &[
    "express", "lodash", "react", "axios", "moment", "chalk", "commander",
    "webpack", "babel-core", "typescript", "eslint", "prettier", "jest",
    "mocha", "vue", "angular", "next", "nuxt", "gatsby", "vite",
    "rollup", "esbuild", "dotenv", "cors", "body-parser", "mongoose",
    "sequelize", "socket.io", "uuid", "crypto-js", "bcrypt", "jsonwebtoken",
    "passport", "multer", "sharp", "nodemailer", "puppeteer", "playwright",
];

const POPULAR_PYPI: &[&str] = &[
    "requests", "numpy", "pandas", "matplotlib", "scipy", "scikit-learn",
    "tensorflow", "torch", "flask", "django", "fastapi", "sqlalchemy",
    "celery", "redis", "boto3", "pydantic", "click", "pillow", "cryptography",
    "paramiko", "fabric", "ansible", "pytest", "black", "mypy", "httpx",
];

const POPULAR_CARGO: &[&str] = &[
    "serde", "tokio", "reqwest", "clap", "anyhow", "thiserror", "tracing",
    "log", "env-logger", "chrono", "uuid", "rand", "rayon", "async-std",
    "actix-web", "axum", "warp", "hyper", "tonic", "prost", "diesel",
    "sqlx", "rusqlite", "redis", "lapin", "crossbeam", "parking-lot",
];

/// Result of a typosquat check.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TyposquatMatch {
    pub candidate:  String,   // the package being checked
    pub target:     String,   // the popular package it resembles
    pub distance:   usize,    // edit distance
    pub ecosystem:  String,
}

/// Check whether `name` in `ecosystem` looks like a typosquat of a popular package.
/// Returns Some(match) if a suspiciously close name is found.
pub fn check(name: &str, ecosystem: &str) -> Option<TyposquatMatch> {
    let candidates = match ecosystem {
        "npm"      => POPULAR_NPM,
        "pypi"     => POPULAR_PYPI,
        "cargo"    => POPULAR_CARGO,
        _          => return None,
    };

    // Normalise: lowercase, strip leading @ and scope.
    let normalised = name
        .to_lowercase()
        .trim_start_matches('@')
        .split('/')
        .last()
        .unwrap_or(name)
        .replace(['-', '_', '.'], "");

    for &popular in candidates {
        // Skip exact matches — those are the real packages.
        if normalised == popular.replace(['-', '_', '.'], "") {
            return None;
        }

        let dist = strsim::levenshtein(&normalised, &popular.replace(['-', '_', '.'], ""));

        // Flag if within edit distance 1 AND names are similar length.
        // Edit distance 1 at short names (< 5 chars) is too noisy — require exact match.
        let min_len = normalised.len().min(popular.len());
        let threshold = if min_len < 5 { 0 } else if min_len < 8 { 1 } else { 2 };

        if dist > 0 && dist <= threshold {
            return Some(TyposquatMatch {
                candidate: name.to_string(),
                target:    popular.to_string(),
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
