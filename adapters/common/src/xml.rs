//! Minimal XML attribute extraction shared by report parsers.
//!
//! These helpers are deliberately small: adapters parse fixed, tool-generated
//! report formats (JUnit, JaCoCo, PIT, Checkstyle). They are not a general
//! XML parser and do not handle namespaces or CDATA.

use std::collections::BTreeMap;

/// Validated, decoded attributes for one XML element. Parse once and reuse for
/// all field lookups; attribute values are never searched as markup.
#[derive(Debug)]
pub struct Attributes {
    values: BTreeMap<String, String>,
}

impl Attributes {
    pub fn parse(mut remaining: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        remaining = remaining.trim();
        while !remaining.is_empty() {
            let (name, value, rest) = next_attribute(remaining)?;
            if values.insert(name.to_string(), decode_xml(value)).is_some() {
                return Err(String::from("invalid or duplicate XML attribute"));
            }
            remaining = rest.trim_start();
        }
        Ok(Self { values })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn string(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }

    pub fn u64(&self, name: &str) -> Option<u64> {
        self.get(name)?.parse().ok()
    }
}

fn next_attribute(input: &str) -> Result<(&str, &str, &str), String> {
    let name_end = input
        .find(|character: char| character.is_whitespace() || character == '=')
        .unwrap_or(input.len());
    let name = &input[..name_end];
    if !is_name(name) {
        return Err(String::from("invalid or duplicate XML attribute"));
    }
    let rest = input[name_end..]
        .trim_start()
        .strip_prefix('=')
        .ok_or_else(|| String::from("XML attribute is missing '='"))?
        .trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|character| matches!(character, '"' | '\''))
        .ok_or_else(|| String::from("XML attribute value must be quoted"))?;
    let rest = &rest[1..];
    let end = rest
        .find(quote)
        .ok_or_else(|| String::from("unterminated XML attribute value"))?;
    let trailing = &rest[end + 1..];
    if !trailing.is_empty() && !trailing.starts_with(char::is_whitespace) {
        return Err(String::from(
            "XML attributes must be separated by whitespace",
        ));
    }
    Ok((name, &rest[..end], trailing))
}

/// Checks the ASCII XML name subset used by supported tool reports.
pub fn is_name(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_alphabetic() || matches!(character, '_' | ':'))
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | ':' | '-' | '.')
        })
}

/// Extracts one attribute. Reuse [`Attributes`] for multiple fields of an element.
pub fn attr_string(attrs: &str, name: &str) -> Option<String> {
    Attributes::parse(attrs).ok()?.string(name)
}

pub fn attr_u64(attrs: &str, name: &str) -> Option<u64> {
    Attributes::parse(attrs).ok()?.u64(name)
}

pub fn attr_f64(attrs: &str, name: &str) -> Option<f64> {
    Attributes::parse(attrs).ok()?.get(name)?.parse().ok()
}

/// Decodes the five predefined XML entities plus numeric character references
/// in a single pass, so sequences like `&amp;lt;` decode to the literal `&lt;`
/// instead of being double-decoded.
pub fn decode_xml(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            decoded.push_str(rest);
            return decoded;
        };
        let entity = &rest[1..semi];
        match decode_entity(entity) {
            Some(replacement) => {
                decoded.push_str(&replacement);
                rest = &rest[semi + 1..];
            }
            None => {
                // Unknown entity: keep the ampersand literal and continue.
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

fn decode_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some(String::from("&")),
        "lt" => Some(String::from("<")),
        "gt" => Some(String::from(">")),
        "quot" => Some(String::from("\"")),
        "apos" => Some(String::from("'")),
        _ => {
            let code = entity.strip_prefix("#x").map_or_else(
                || entity.strip_prefix('#')?.parse::<u32>().ok(),
                |hex| u32::from_str_radix(hex, 16).ok(),
            )?;
            char::from_u32(code).map(String::from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{attr_f64, attr_string, attr_u64, decode_xml};

    #[test]
    fn parses_quoted_attributes_once_without_matching_inside_values() {
        let attrs =
            super::Attributes::parse(r#"message='name="fake" &amp;lt;' name = "real" count='12'"#)
                .unwrap();
        assert_eq!(attrs.get("name"), Some("real"));
        assert_eq!(attrs.get("message"), Some("name=\"fake\" &lt;"));
        assert_eq!(attrs.u64("count"), Some(12));
        assert_eq!(attrs.get("missing"), None);
        assert_eq!(
            attr_string("name='single quoted'", "name").as_deref(),
            Some("single quoted")
        );
    }

    #[test]
    fn rejects_malformed_and_duplicate_attributes() {
        for attrs in [
            "name",
            "name=value",
            "name='unfinished",
            "name='a' name='b'",
            "name='a'other='b'",
            "1name='a'",
        ] {
            assert!(super::Attributes::parse(attrs).is_err(), "{attrs}");
        }
    }

    #[test]
    fn extracts_attributes() {
        let attrs = r#"name="suite" tests="12" time="1.5""#;
        assert_eq!(attr_string(attrs, "name").as_deref(), Some("suite"));
        assert_eq!(attr_u64(attrs, "tests"), Some(12));
        assert_eq!(attr_f64(attrs, "time"), Some(1.5));
        assert_eq!(attr_string(attrs, "missing"), None);
    }

    #[test]
    fn decodes_predefined_entities() {
        assert_eq!(decode_xml("a &lt; b &amp;&amp; c &gt; d"), "a < b && c > d");
        assert_eq!(decode_xml("&quot;x&quot; &apos;y&apos;"), "\"x\" 'y'");
    }

    #[test]
    fn does_not_double_decode_escaped_entities() {
        // `&amp;lt;` is the literal text `&lt;`, not `<`.
        assert_eq!(decode_xml("&amp;lt;tag&amp;gt;"), "&lt;tag&gt;");
    }

    #[test]
    fn decodes_numeric_references() {
        assert_eq!(decode_xml("caf&#233;"), "café");
        assert_eq!(decode_xml("caf&#xE9;"), "café");
    }

    #[test]
    fn keeps_unknown_entities_and_bare_ampersands() {
        assert_eq!(decode_xml("a & b"), "a & b");
        assert_eq!(decode_xml("&unknown;"), "&unknown;");
        assert_eq!(decode_xml("trailing &"), "trailing &");
    }
}
