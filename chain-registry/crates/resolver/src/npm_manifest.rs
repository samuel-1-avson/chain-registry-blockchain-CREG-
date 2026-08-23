// crates/resolver/src/npm_manifest.rs
// Direct npm dependency names for PATH-shim lockfile sync (bare `npm install`
// / `npm ci`). Transitive lockfile entries are not gated here; that would
// refuse almost every real project that is not fully published to CREG.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const MAX_DIRECT_DEPS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmDirectDep {
    pub name: String,
    pub version: String,
}

impl NpmDirectDep {
    pub fn install_spec(&self) -> String {
        if self.version.is_empty() || self.version == "latest" {
            self.name.clone()
        } else {
            format!("{}@{}", self.name, self.version)
        }
    }
}

pub fn npm_include_dev_dependencies(args: &[String]) -> bool {
    let omits_dev = args.iter().any(|arg| {
        arg == "--production"
            || arg == "--omit=dev"
            || arg
                .strip_prefix("--omit=")
                .is_some_and(|list| list.split(',').any(|item| item.trim() == "dev"))
    });
    !omits_dev
}

pub fn read_npm_direct_deps(dir: &Path, include_dev: bool) -> Result<Vec<NpmDirectDep>> {
    let package_json = dir.join("package.json");
    if !package_json.exists() {
        bail!(
            "no package.json in {} — refusing ungated `npm install`/`npm ci`. \
             Use `creg install <pkg>` or set CREG_ALLOW_UNVERIFIED=true only as break-glass.",
            dir.display()
        );
    }
    let raw = std::fs::read_to_string(&package_json)
        .with_context(|| format!("cannot read {}", package_json.display()))?;
    let manifest: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", package_json.display()))?;

    let mut names: BTreeMap<String, String> = BTreeMap::new();
    collect_dep_map(&manifest, "dependencies", &mut names);
    collect_dep_map(&manifest, "optionalDependencies", &mut names);
    if include_dev {
        collect_dep_map(&manifest, "devDependencies", &mut names);
    }

    if names.len() > MAX_DIRECT_DEPS {
        bail!(
            "package.json lists {} direct dependencies (max {MAX_DIRECT_DEPS}). \
             Refusing ungated bulk install; use CREG_ALLOW_UNVERIFIED=true only as break-glass.",
            names.len()
        );
    }

    let lock = read_lockfile(dir);
    let mut deps = Vec::with_capacity(names.len());
    for (name, declared) in names {
        let version = lock
            .as_ref()
            .and_then(|lock| lockfile_exact_version(lock, &name))
            .unwrap_or(declared);
        deps.push(NpmDirectDep { name, version });
    }
    Ok(deps)
}

fn collect_dep_map(manifest: &Value, field: &str, out: &mut BTreeMap<String, String>) {
    let Some(map) = manifest.get(field).and_then(Value::as_object) else {
        return;
    };
    for (name, spec) in map {
        let name = name.trim();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let version = spec.as_str().unwrap_or("latest").trim();
        out.insert(
            name.to_string(),
            if version.is_empty() {
                "latest".into()
            } else {
                version.to_string()
            },
        );
    }
}

fn read_lockfile(dir: &Path) -> Option<Value> {
    let path = dir.join("package-lock.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn lockfile_exact_version(lock: &Value, name: &str) -> Option<String> {
    let v2_key = format!("node_modules/{name}");
    if let Some(version) = lock
        .get("packages")
        .and_then(Value::as_object)
        .and_then(|packages| packages.get(&v2_key))
        .and_then(|entry| entry.get("version"))
        .and_then(Value::as_str)
    {
        return Some(version.to_string());
    }
    lock.get("dependencies")
        .and_then(Value::as_object)
        .and_then(|deps| deps.get(name))
        .and_then(|entry| entry.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).expect("write fixture");
    }

    #[test]
    fn missing_package_json_fails_closed() {
        let tmp = TempDir::new().unwrap();
        let err = read_npm_direct_deps(tmp.path(), true).unwrap_err();
        assert!(err.to_string().contains("no package.json"));
    }

    #[test]
    fn reads_direct_deps_and_prefers_lockfile_exact_version() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "package.json",
            r#"{
              "dependencies": { "lodash": "^4.17.21", "@scope/pkg": "1.0.0" },
              "devDependencies": { "jest": "^29.0.0" }
            }"#,
        );
        write(
            tmp.path(),
            "package-lock.json",
            r#"{
              "packages": {
                "": {},
                "node_modules/lodash": { "version": "4.17.21" },
                "node_modules/@scope/pkg": { "version": "1.0.0" },
                "node_modules/jest": { "version": "29.7.0" }
              }
            }"#,
        );
        let with_dev = read_npm_direct_deps(tmp.path(), true).unwrap();
        assert_eq!(
            with_dev
                .iter()
                .map(|d| d.install_spec())
                .collect::<Vec<_>>(),
            vec!["@scope/pkg@1.0.0", "jest@29.7.0", "lodash@4.17.21"]
        );
        let prod_only = read_npm_direct_deps(tmp.path(), false).unwrap();
        assert_eq!(
            prod_only
                .iter()
                .map(|d| d.install_spec())
                .collect::<Vec<_>>(),
            vec!["@scope/pkg@1.0.0", "lodash@4.17.21"]
        );
    }

    #[test]
    fn lockfile_v1_dependencies_map_is_accepted() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "package.json",
            r#"{ "dependencies": { "left-pad": "^1.3.0" } }"#,
        );
        write(
            tmp.path(),
            "package-lock.json",
            r#"{ "dependencies": { "left-pad": { "version": "1.3.0" } } }"#,
        );
        let deps = read_npm_direct_deps(tmp.path(), true).unwrap();
        assert_eq!(deps[0].install_spec(), "left-pad@1.3.0");
    }

    #[test]
    fn empty_manifest_is_ok() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{ "name": "empty" }"#);
        assert!(read_npm_direct_deps(tmp.path(), true).unwrap().is_empty());
    }

    #[test]
    fn too_many_direct_deps_fail_closed() {
        let tmp = TempDir::new().unwrap();
        let mut deps = String::from("{");
        for i in 0..=MAX_DIRECT_DEPS {
            if i > 0 {
                deps.push(',');
            }
            deps.push_str(&format!("\"pkg-{i}\": \"1.0.0\""));
        }
        deps.push('}');
        write(
            tmp.path(),
            "package.json",
            &format!(r#"{{ "dependencies": {deps} }}"#),
        );
        let err = read_npm_direct_deps(tmp.path(), false).unwrap_err();
        assert!(err.to_string().contains("direct dependencies"));
    }

    #[test]
    fn production_flag_omits_dev_dependencies() {
        assert!(npm_include_dev_dependencies(&[
            "npm".into(),
            "install".into()
        ]));
        assert!(!npm_include_dev_dependencies(&[
            "npm".into(),
            "install".into(),
            "--production".into()
        ]));
        assert!(!npm_include_dev_dependencies(&[
            "npm".into(),
            "ci".into(),
            "--omit=dev".into()
        ]));
    }
}
