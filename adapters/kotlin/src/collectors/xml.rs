use ayni_adapters_common::xml::{Attributes, is_name};

pub(super) struct XmlElement {
    pub(super) name: String,
    pub(super) attrs: Attributes,
    pub(super) content_start: usize,
    pub(super) content_end: usize,
    pub(super) parent: Option<usize>,
    start: usize,
    end: usize,
}

pub(super) struct XmlDocument {
    pub(super) elements: Vec<XmlElement>,
}

impl XmlDocument {
    pub(super) fn parse(content: &str) -> Result<Self, String> {
        let mut elements = Vec::new();
        let mut open = Vec::new();
        let mut offset = 0;
        while let Some(relative_start) = content[offset..].find('<') {
            let start = offset + relative_start;
            offset = parse_markup(content, start, &mut elements, &mut open)?;
        }
        if let Some(index) = open.last() {
            return Err(format!("unclosed <{}> element", elements[*index].name));
        }
        let roots = elements
            .iter()
            .filter(|element| element.parent.is_none())
            .collect::<Vec<_>>();
        if roots.len() != 1 {
            return Err(String::from(
                "XML document must contain exactly one root element",
            ));
        }
        let root = roots[0];
        if !is_misc(&content[..root.start]) || !is_misc(&content[root.end..]) {
            return Err(String::from(
                "XML document contains text outside its root element",
            ));
        }
        Ok(Self { elements })
    }

    pub(super) fn has_ancestor_index(&self, mut index: usize, ancestor: usize) -> bool {
        while let Some(parent) = self.elements[index].parent {
            if parent == ancestor {
                return true;
            }
            index = parent;
        }
        false
    }

    pub(super) fn text(&self, content: &str, element: &XmlElement) -> String {
        strip_markup(&content[element.content_start..element.content_end])
    }
}

fn parse_markup(
    content: &str,
    start: usize,
    elements: &mut Vec<XmlElement>,
    open: &mut Vec<usize>,
) -> Result<usize, String> {
    let rest = &content[start..];
    if rest.starts_with("<!--") {
        return consume_until(content, start + 4, "-->", "comment");
    }
    if rest.starts_with("<![CDATA[") {
        return consume_until(content, start + 9, "]]>", "CDATA section");
    }
    if rest.starts_with("<?") {
        return consume_until(content, start + 2, "?>", "processing instruction");
    }
    if rest.starts_with("</") {
        return close_element(content, start, elements, open);
    }
    if rest.starts_with("<!DOCTYPE") {
        return tag_end(content, start).map(|end| end + 1);
    }
    if rest.starts_with("<!") {
        return Err(String::from("unsupported XML declaration"));
    }
    open_element(content, start, elements, open)
}

fn consume_until(
    content: &str,
    offset: usize,
    terminator: &str,
    kind: &str,
) -> Result<usize, String> {
    content[offset..]
        .find(terminator)
        .map(|end| offset + end + terminator.len())
        .ok_or_else(|| format!("unterminated XML {kind}"))
}

fn open_element(
    content: &str,
    start: usize,
    elements: &mut Vec<XmlElement>,
    open: &mut Vec<usize>,
) -> Result<usize, String> {
    let end = tag_end(content, start)?;
    let tag = &content[start + 1..end];
    let self_closing = tag.trim_end().ends_with('/');
    let tag = tag.trim_end().strip_suffix('/').unwrap_or(tag.trim_end());
    let (name, attrs) = split_name_and_attrs(tag)?;
    let attrs = Attributes::parse(attrs)?;
    let index = elements.len();
    elements.push(XmlElement {
        name: name.to_string(),
        attrs,
        content_start: end + 1,
        content_end: end + 1,
        parent: open.last().copied(),
        start,
        end: end + 1,
    });
    if !self_closing {
        open.push(index);
    }
    Ok(end + 1)
}

fn close_element(
    content: &str,
    start: usize,
    elements: &mut [XmlElement],
    open: &mut Vec<usize>,
) -> Result<usize, String> {
    let end = tag_end(content, start)?;
    let name = content[start + 2..end].trim();
    if !is_name(name) {
        return Err(String::from("invalid XML closing tag"));
    }
    let index = open
        .pop()
        .ok_or_else(|| format!("unexpected closing </{name}> element"))?;
    if elements[index].name != name {
        return Err(format!(
            "mismatched closing </{name}> element for <{}>",
            elements[index].name
        ));
    }
    elements[index].content_end = start;
    elements[index].end = end + 1;
    Ok(end + 1)
}

fn tag_end(content: &str, start: usize) -> Result<usize, String> {
    let mut quote = None;
    for (relative, character) in content[start + 1..].char_indices() {
        match (quote, character) {
            (Some(current), character) if current == character => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '>') => return Ok(start + 1 + relative),
            _ => {}
        }
    }
    Err(String::from("unterminated XML tag"))
}

fn split_name_and_attrs(tag: &str) -> Result<(&str, &str), String> {
    let split = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let (name, attrs) = tag.split_at(split);
    if !is_name(name) {
        return Err(String::from("invalid XML element name"));
    }
    Ok((name, attrs))
}

fn is_misc(mut content: &str) -> bool {
    loop {
        content = content.trim_start();
        if content.is_empty() {
            return true;
        }
        if content.starts_with("<!DOCTYPE") {
            let Ok(end) = tag_end(content, 0) else {
                return false;
            };
            content = &content[end + 1..];
            continue;
        }
        let (prefix, suffix) = if content.starts_with("<!--") {
            ("<!--", "-->")
        } else if content.starts_with("<?") {
            ("<?", "?>")
        } else {
            return false;
        };
        let Some(end) = content[prefix.len()..].find(suffix) else {
            return false;
        };
        content = &content[prefix.len() + end + suffix.len()..];
    }
}

fn strip_markup(content: &str) -> String {
    let mut text = String::new();
    let mut remaining = content;
    while let Some(start) = remaining.find('<') {
        text.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find('>') else {
            break;
        };
        remaining = &remaining[start + end + 1..];
    }
    text.push_str(remaining);
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::XmlDocument;

    #[test]
    fn accepts_a_document_with_declaration_and_doctype() {
        assert!(
            XmlDocument::parse(
                "<?xml version=\"1.0\"?><!DOCTYPE report SYSTEM \"report.dtd\"><report/>"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_unbalanced_and_mismatched_xml() {
        for content in [
            "<report><counter/>",
            "<report><counter></report>",
            "<report/>trailing text",
        ] {
            assert!(XmlDocument::parse(content).is_err(), "{content}");
        }
    }
}
