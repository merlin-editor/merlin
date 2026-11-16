//! CMake's file API: the plugin asks for the code model before a
//! configure, and reads each target from CMake's reply after it. See
//! cmake-file-api(7).

use serde_json::Value;

/// The folder the plugin's query goes in, under the build folder.
const QUERY: &str = ".cmake/api/v1/query/client-merlin";
const REPLY: &str = ".cmake/api/v1/reply";

/// A target the build makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    /// `EXECUTABLE`, `STATIC_LIBRARY` and so on.
    pub kind: String,
    /// What it builds, relative to the workspace or absolute: a program
    /// for an executable.
    pub artifact: Option<String>,
}

impl Target {
    pub fn is_program(&self) -> bool {
        self.kind == "EXECUTABLE"
    }
}

fn join(dir: &str, path: &str) -> String {
    let absolute = path.starts_with('/') || matches!(path.get(1..3), Some(":/" | ":\\"));
    if absolute || dir == "." || dir.is_empty() {
        path.to_string()
    } else {
        format!("{}/{path}", dir.trim_end_matches('/'))
    }
}

/// Commands that ask CMake for the code model in `dir`, to run before a
/// configure, joined with `&&`.
pub fn query(dir: &str) -> String {
    let folder = join(dir, QUERY);
    format!(
        "cmake -E make_directory {} && cmake -E touch {}",
        quote(&folder),
        quote(&format!("{folder}/codemodel-v2"))
    )
}

fn quote(path: &str) -> String {
    if path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        path.to_string()
    } else {
        format!("\"{path}\"")
    }
}

/// The targets in `dir`'s newest reply, if CMake answered the query:
/// `list` lists a folder, `read` reads a file, both from the workspace.
pub fn targets(
    dir: &str,
    list: impl Fn(&str) -> Option<Vec<String>>,
    read: impl Fn(&str) -> Option<String>,
) -> Option<Vec<Target>> {
    let reply = join(dir, REPLY);
    // Index files are named for when they were written, so the last is
    // the newest.
    let index = list(&reply)?
        .into_iter()
        .filter(|n| n.starts_with("index-") && n.ends_with(".json"))
        .max()?;
    let index: Value = serde_json::from_str(&read(&format!("{reply}/{index}"))?).ok()?;
    let model = index["reply"]["client-merlin"]["codemodel-v2"]["jsonFile"].as_str()?;
    let model: Value = serde_json::from_str(&read(&format!("{reply}/{model}"))?).ok()?;
    let configuration = model["configurations"].as_array()?.first()?;
    let mut targets = Vec::new();
    for entry in configuration["targets"].as_array().into_iter().flatten() {
        let Some(file) = entry["jsonFile"].as_str() else {
            continue;
        };
        let Some(target) =
            read(&format!("{reply}/{file}")).and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let kind = target["type"].as_str().unwrap_or_default().to_string();
        // UTILITY is a custom target; INTERFACE builds nothing.
        if matches!(kind.as_str(), "UTILITY" | "INTERFACE_LIBRARY") {
            continue;
        }
        let artifact = target["artifacts"][0]["path"].as_str().map(|p| {
            // Artifacts are relative to the top of the build tree, which
            // the model gives absolute; keep the workspace's own form.
            if p.starts_with('/') {
                p.to_string()
            } else {
                join(dir, p)
            }
        });
        targets.push(Target {
            name: target["name"].as_str().unwrap_or_default().to_string(),
            kind,
            artifact,
        });
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    Some(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn the_query_goes_in_the_build_folder() {
        assert_eq!(
            query("out/debug"),
            "cmake -E make_directory out/debug/.cmake/api/v1/query/client-merlin \
             && cmake -E touch out/debug/.cmake/api/v1/query/client-merlin/codemodel-v2"
        );
    }

    #[test]
    fn targets_come_from_the_newest_reply() {
        let reply = "build/.cmake/api/v1/reply";
        let files: BTreeMap<String, &str> = [
            (
                "index-2026-11-17T01-00-00-0000.json",
                r#"{"reply": {"client-merlin": {"codemodel-v2": {"jsonFile": "codemodel-v2-new.json"}}}}"#,
            ),
            ("index-2026-11-16T01-00-00-0000.json", r#"{"reply": {}}"#),
            (
                "codemodel-v2-new.json",
                r#"{"paths": {"build": "/w/build", "source": "/w"},
                    "configurations": [{"name": "Debug", "targets": [
                        {"name": "app", "jsonFile": "target-app.json"},
                        {"name": "shapes", "jsonFile": "target-shapes.json"},
                        {"name": "docs", "jsonFile": "target-docs.json"}
                    ]}]}"#,
            ),
            (
                "target-app.json",
                r#"{"name": "app", "type": "EXECUTABLE", "artifacts": [{"path": "app"}]}"#,
            ),
            (
                "target-shapes.json",
                r#"{"name": "shapes", "type": "STATIC_LIBRARY", "artifacts": [{"path": "libshapes.a"}]}"#,
            ),
            ("target-docs.json", r#"{"name": "docs", "type": "UTILITY"}"#),
        ]
        .into_iter()
        .map(|(n, t)| (format!("{reply}/{n}"), t))
        .collect();
        let list = |dir: &str| {
            (dir == reply).then(|| {
                files
                    .keys()
                    .map(|k| k.rsplit('/').next().unwrap().to_string())
                    .collect()
            })
        };
        let read = |path: &str| files.get(path).map(|t| t.to_string());
        let found = targets("build", list, read).unwrap();
        assert_eq!(
            found,
            [
                Target {
                    name: "app".into(),
                    kind: "EXECUTABLE".into(),
                    artifact: Some("build/app".into())
                },
                Target {
                    name: "shapes".into(),
                    kind: "STATIC_LIBRARY".into(),
                    artifact: Some("build/libshapes.a".into())
                },
            ]
        );
        assert!(found[0].is_program() && !found[1].is_program());
        assert_eq!(targets("other", list, read), None, "no reply there");
    }
}
