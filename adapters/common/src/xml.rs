//! Minimal XML attribute extraction shared by report parsers.
//!
//! These helpers are deliberately small: adapters parse fixed, tool-generated
//! report formats (JUnit, JaCoCo, PIT, Checkstyle). They are not a general
//! XML parser and do not handle namespaces or CDATA.

use regex::Regex;

/// Extracts a quoted attribute value (`name="value"`) from an attribute string,
/// decoding XML entities.
pub fn attr_string(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"(?:^|\s){name}\s*=\s*"([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    re.captures(attrs)
        .and_then(|caps| caps.get(1))
        .map(|value| decode_xml(value.as_str()))
}

/// [`attr_string`] parsed as `u64`.
pub fn attr_u64(attrs: &str, name: &str) -> Option<u64> {
    attr_string(attrs, name).and_then(|value| value.parse::<u64>().ok())
}

/// [`attr_string`] parsed as `f64`.
pub fn attr_f64(attrs: &str, name: &str) -> Option<f64> {
    attr_string(attrs, name).and_then(|value| value.parse::<f64>().ok())
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
