const MAX_NODES: usize = 48;
const BROWSER_TEXT_CAPACITY: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Title,
    Heading,
    Paragraph,
    Link,
    ListItem,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserText {
    bytes: [u8; BROWSER_TEXT_CAPACITY],
    length: u8,
}

impl BrowserText {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; BROWSER_TEXT_CAPACITY],
            length: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, BrowserError> {
        let value = value.trim();
        if value.is_empty() || !value.is_ascii() {
            return Err(BrowserError::InvalidDocument);
        }
        let mut text = Self::empty();
        let length = value.len().min(BROWSER_TEXT_CAPACITY);
        text.bytes[..length].copy_from_slice(&value.as_bytes()[..length]);
        text.length = length as u8;
        Ok(text)
    }

    pub fn as_str(&self) -> &str {
        // BrowserText::new only accepts ASCII.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.length as usize]) }
    }
}

impl core::fmt::Debug for BrowserText {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("BrowserText")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentNode {
    pub kind: NodeKind,
    pub text: BrowserText,
    pub target: BrowserText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserError {
    InvalidUrl,
    InvalidDocument,
    TooManyNodes,
}

pub struct Document {
    url: BrowserText,
    nodes: [Option<DocumentNode>; MAX_NODES],
    count: usize,
}

impl Document {
    pub fn parse(url: &str, source: &str) -> Result<Self, BrowserError> {
        if !url.starts_with("hexa://")
            && !url.starts_with("data:text/html,")
            && !url.starts_with("http://")
        {
            return Err(BrowserError::InvalidUrl);
        }
        if !source.is_ascii() {
            return Err(BrowserError::InvalidDocument);
        }
        let mut document = Self {
            url: BrowserText::new(url)?,
            nodes: [None; MAX_NODES],
            count: 0,
        };
        let mut cursor = 0;
        while let Some(relative_start) = source[cursor..].find('<') {
            let tag_start = cursor + relative_start;
            let Some(relative_end) = source[tag_start..].find('>') else {
                break;
            };
            let tag_end = tag_start + relative_end;
            let tag = &source[tag_start + 1..tag_end];
            cursor = tag_end + 1;
            if tag.starts_with('/') || tag.starts_with('!') {
                continue;
            }
            let Some(kind) = node_kind(tag) else { continue };
            let content_end = source[cursor..]
                .find('<')
                .map(|offset| cursor + offset)
                .unwrap_or(source.len());
            let content = source[cursor..content_end].trim();
            if content.is_empty() {
                continue;
            }
            let target = if kind == NodeKind::Link {
                attribute(tag, "href")
                    .and_then(|value| BrowserText::new(value).ok())
                    .unwrap_or(BrowserText::empty())
            } else {
                BrowserText::empty()
            };
            document.push(DocumentNode {
                kind,
                text: BrowserText::new(content)?,
                target,
            })?;
        }
        if document.count == 0 {
            return Err(BrowserError::InvalidDocument);
        }
        Ok(document)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &DocumentNode> {
        self.nodes.iter().flatten()
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    fn push(&mut self, node: DocumentNode) -> Result<(), BrowserError> {
        let slot = self
            .nodes
            .get_mut(self.count)
            .ok_or(BrowserError::TooManyNodes)?;
        *slot = Some(node);
        self.count += 1;
        Ok(())
    }
}

fn node_kind(tag: &str) -> Option<NodeKind> {
    let name = tag.split_ascii_whitespace().next().unwrap_or("");
    match name {
        "title" => Some(NodeKind::Title),
        "h1" | "h2" | "h3" => Some(NodeKind::Heading),
        "p" => Some(NodeKind::Paragraph),
        "a" => Some(NodeKind::Link),
        "li" => Some(NodeKind::ListItem),
        _ => None,
    }
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = name;
    let start = tag.find(marker)? + marker.len();
    let rest = tag[start..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.as_bytes().first().copied()?;
    if quote != b'\'' && quote != b'"' {
        return rest.split_ascii_whitespace().next();
    }
    let value = &rest[1..];
    let end = value.find(quote as char)?;
    Some(&value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_safe_local_document_and_links() {
        let document = Document::parse(
            "hexa://home",
            "<title>Hexa Home</title><h1>Welcome</h1><p>Forms make the environment.</p><a href='hexa://about'>About</a>",
        )
        .unwrap();
        assert_eq!(document.url(), "hexa://home");
        assert_eq!(document.len(), 4);
        let link = document.nodes().last().unwrap();
        assert_eq!(link.kind, NodeKind::Link);
        assert_eq!(link.target.as_str(), "hexa://about");
    }

    #[test]
    fn accepts_http_documents_but_not_unimplemented_https() {
        let document = Document::parse(
            "http://example.com/",
            "<title>Example</title><h1>Example Domain</h1><p>Network document.</p>",
        )
        .unwrap();
        assert_eq!(document.url(), "http://example.com/");
        assert_eq!(
            Document::parse("https://example.com", "<p>Nope</p>").err(),
            Some(BrowserError::InvalidUrl)
        );
    }
}
