//! A build's `CMakeCache.txt`, as the CMake view lists it: each variable
//! the project or CMake keeps for the user, its type, value and help, and
//! whether CMake calls it advanced.

/// One cache variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    /// `BOOL`, `STRING`, `PATH`, `FILEPATH`, or `UNINITIALIZED` for a `-D`
    /// without a type.
    pub kind: String,
    pub value: String,
    /// The `//` lines over it.
    pub help: String,
    /// CMake marks it advanced, so it's hidden unless asked.
    pub advanced: bool,
    /// The values its `STRINGS` property offers, for a choice.
    pub choices: Vec<String>,
}

impl Variable {
    /// A boolean, or one set by `-D` without a type to ON or OFF.
    pub fn is_bool(&self) -> bool {
        self.kind == "BOOL"
            || (self.kind == "UNINITIALIZED"
                && matches!(
                    self.value.to_ascii_uppercase().as_str(),
                    "ON" | "OFF" | "TRUE" | "FALSE" | "YES" | "NO"
                ))
    }

    pub fn on(&self) -> bool {
        matches!(
            self.value.to_ascii_uppercase().as_str(),
            "ON" | "TRUE" | "YES" | "Y" | "1"
        )
    }
}

/// The variables in a cache, A to Z, without CMake's internal ones.
pub fn variables(text: &str) -> Vec<Variable> {
    let mut found: Vec<Variable> = Vec::new();
    let mut advanced: Vec<String> = Vec::new();
    let mut strings: Vec<(String, Vec<String>)> = Vec::new();
    let mut help: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(text) = line.strip_prefix("//") {
            help.push(text.trim().to_string());
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            help.clear();
            continue;
        }
        let Some((left, value)) = line.split_once('=') else {
            help.clear();
            continue;
        };
        let Some((name, kind)) = left.rsplit_once(':') else {
            help.clear();
            continue;
        };
        // A property, as `NAME-ADVANCED:INTERNAL=1`.
        if kind == "INTERNAL" {
            if let Some(owner) = name.strip_suffix("-ADVANCED") {
                if value.trim() == "1" {
                    advanced.push(owner.to_string());
                }
            } else if let Some(owner) = name.strip_suffix("-STRINGS") {
                strings.push((
                    owner.to_string(),
                    value.split(';').map(str::to_string).collect(),
                ));
            }
            help.clear();
            continue;
        }
        if kind == "STATIC" {
            help.clear();
            continue;
        }
        found.push(Variable {
            name: name.to_string(),
            kind: kind.to_string(),
            value: value.to_string(),
            help: help.join(" "),
            advanced: false,
            choices: Vec::new(),
        });
        help.clear();
    }
    for v in &mut found {
        v.advanced = advanced.contains(&v.name);
        if let Some((_, choices)) = strings.iter().find(|(n, _)| *n == v.name) {
            v.choices = choices.clone();
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_with_their_kinds_help_and_choices() {
        let cache = "# This is the CMakeCache file.\n\
            //Build tests\n\
            BUILD_TESTING:BOOL=ON\n\
            \n\
            //Choose the type of build.\n\
            CMAKE_BUILD_TYPE:STRING=Debug\n\
            CMAKE_AR:FILEPATH=/usr/bin/ar\n\
            CMAKE_EXPORT_COMPILE_COMMANDS:UNINITIALIZED=ON\n\
            CMAKE_AR-ADVANCED:INTERNAL=1\n\
            CMAKE_BUILD_TYPE-STRINGS:INTERNAL=Debug;Release\n\
            CMAKE_CACHEFILE_DIR:INTERNAL=/w/build\n\
            shapes_BINARY_DIR:STATIC=/w/build\n";
        let found = variables(cache);
        let names: Vec<&str> = found.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "BUILD_TESTING",
                "CMAKE_AR",
                "CMAKE_BUILD_TYPE",
                "CMAKE_EXPORT_COMPILE_COMMANDS"
            ]
        );
        assert!(found[0].is_bool() && found[0].on());
        assert!(found[3].is_bool(), "a -D ON without a type");
        assert!(!found[2].is_bool());
        assert_eq!(found[0].help, "Build tests");
        assert!(found[1].advanced);
        assert_eq!(found[2].choices, ["Debug", "Release"]);
        assert_eq!(found[2].help, "Choose the type of build.");
    }
}
