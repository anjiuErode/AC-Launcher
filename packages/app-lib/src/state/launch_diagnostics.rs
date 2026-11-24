use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchIssue {
    Java,
    MissingMod { mod_id: String },
    GameLibrary,
    UnknownExit,
}

#[derive(Clone, Debug, Serialize)]
pub struct LaunchDiagnostic {
    pub run_id: Uuid,
    pub exit_code: Option<i32>,
    pub issue: LaunchIssue,
}

pub(super) fn diagnose_launch_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<LaunchIssue> {
    static FABRIC_MISSING_MOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\brequires\b[^\r\n]{0,512}\(([a-z][a-z0-9_-]{1,63})\),?\s+which is missing")
            .expect("valid Fabric dependency pattern")
    });
    static FORGE_MISSING_MOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\bmod id:\s*['"]([a-z][a-z0-9_-]{1,63})['"][^\r\n]{0,512}\bactual version:\s*['"]?\[missing\]"#)
            .expect("valid Forge dependency pattern")
    });

    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.contains("unsupportedclassversionerror")
            || lower.contains("only recognizes class file versions up to")
            || lower.contains("missing correct java installation")
            || lower.contains("no java version found for java version")
            || lower.contains("a java runtime environment (jre) or java development kit")
            || (lower.contains("requires java") && lower.contains("running java"))
        {
            return Some(LaunchIssue::Java);
        }

        let missing_mod = FABRIC_MISSING_MOD
            .captures(&lower)
            .or_else(|| FORGE_MISSING_MOD.captures(&lower))
            .and_then(|value| value.get(1).map(|value| value.as_str()));
        if let Some(mod_id) = missing_mod {
            if mod_id == "java" {
                return Some(LaunchIssue::Java);
            }
            if !matches!(mod_id, "minecraft" | "fabricloader" | "forge" | "neoforge" | "quilt_loader") {
                return Some(LaunchIssue::MissingMod { mod_id: mod_id.into() });
            }
        }

        if lower.contains("incorrect sha1 hash for download")
            || ((lower.contains("invalid or corrupt jarfile")
                || lower.contains("unable to access jarfile")
                || lower.contains("zipexception")
                || lower.contains("nosuchfileexception"))
                && (lower.contains("libraries/") || lower.contains("libraries\\")))
        {
            return Some(LaunchIssue::GameLibrary);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{LaunchIssue, diagnose_launch_lines};

    #[test]
    fn recognizes_specific_launch_failures_without_exposing_raw_logs() {
        assert_eq!(
            diagnose_launch_lines(["java.lang.UnsupportedClassVersionError: example.Main"]),
            Some(LaunchIssue::Java)
        );
        assert_eq!(
            diagnose_launch_lines(["Mod 'Example' (example) requires any version of Fabric API (fabric-api), which is missing!"]),
            Some(LaunchIssue::MissingMod { mod_id: "fabric-api".into() })
        );
        assert_eq!(
            diagnose_launch_lines(["Mod ID: 'architectury', Requested by: 'example', Expected range: '[1,)', Actual version: '[MISSING]'"]),
            Some(LaunchIssue::MissingMod { mod_id: "architectury".into() })
        );
        assert_eq!(
            diagnose_launch_lines([r"Invalid or corrupt jarfile D:\game\libraries\example.jar"]),
            Some(LaunchIssue::GameLibrary)
        );
        assert_eq!(
            diagnose_launch_lines(["ZipException: mods/example.jar", "Closing normally"]),
            None
        );
    }
}
