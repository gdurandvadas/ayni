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
}

impl Language {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Node => "node",
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
            _ => Err(format!("unsupported language: {value}")),
        }
    }
}
