use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Languages supported by Ayni adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Go,
    Node,
    Python,
    Kotlin,
}

impl Language {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Node => "node",
            Self::Python => "python",
            Self::Kotlin => "kotlin",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Language {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "rust" => Ok(Self::Rust),
            "go" => Ok(Self::Go),
            "node" | "nodejs" | "javascript" | "typescript" => Ok(Self::Node),
            "python" | "py" => Ok(Self::Python),
            "kotlin" | "kt" => Ok(Self::Kotlin),
            _ => Err(format!("unsupported language: {value}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Language;
    use std::str::FromStr;

    #[test]
    fn parses_python_aliases() {
        assert_eq!(Language::from_str("python"), Ok(Language::Python));
        assert_eq!(Language::from_str("py"), Ok(Language::Python));
        assert_eq!(Language::Python.as_str(), "python");
    }

    #[test]
    fn parses_kotlin_aliases() {
        assert_eq!(Language::from_str("kotlin"), Ok(Language::Kotlin));
        assert_eq!(Language::from_str("kt"), Ok(Language::Kotlin));
        assert_eq!(Language::Kotlin.as_str(), "kotlin");
    }
}
