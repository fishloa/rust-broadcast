#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn has_attr(&self, key: &str) -> bool {
        self.attrs.iter().any(|(k, _)| k == key)
    }
    /// Direct children with the given tag name.
    pub fn find_all<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        self.children.iter().filter(|c| c.name == name).collect()
    }
    pub fn find<'a>(&'a self, name: &str) -> Option<&'a Element> {
        self.children.iter().find(|c| c.name == name)
    }
    /// Recursive descendant search (first match, DFS).
    pub fn descendant<'a>(&'a self, name: &str) -> Option<&'a Element> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(d) = c.descendant(name) {
                return Some(d);
            }
        }
        None
    }
    /// Collect the names of every element in the tree (self + descendants).
    pub fn all_names(&self, out: &mut std::collections::BTreeSet<String>) {
        out.insert(self.name.clone());
        for c in &self.children {
            c.all_names(out);
        }
    }
}

/// Parse `s` with `quick-xml` into an [`Element`] tree, asserting the document
/// is well-formed: matched tags (checked by the reader), a single root element,
/// and nothing but whitespace outside it.
pub fn parse_xml(s: &str) -> Element {
    use quick_xml::Reader;
    use quick_xml::XmlVersion;
    use quick_xml::events::{BytesStart, Event};

    fn open(e: &BytesStart<'_>) -> Element {
        let attrs = e
            .attributes()
            .map(|a| {
                let a = a.expect("well-formed attribute");
                let v = a
                    .normalized_value(XmlVersion::Implicit1_0)
                    .expect("resolvable attribute value");
                (a.key.as_ref().to_string(), v.into_owned())
            })
            .collect();
        Element {
            name: e.name().as_ref().to_string(),
            attrs,
            children: Vec::new(),
        }
    }

    let mut reader = Reader::from_str(s);
    // Bottom of the stack is a synthetic document node holding the root.
    let mut stack = vec![Element {
        name: String::new(),
        attrs: Vec::new(),
        children: Vec::new(),
    }];
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) => stack.push(open(&e)),
            Event::Empty(e) => {
                let el = open(&e);
                stack.last_mut().expect("document node").children.push(el);
            }
            Event::End(_) => {
                let el = stack.pop().expect("open element");
                stack.last_mut().expect("document node").children.push(el);
            }
            Event::Text(t) if stack.len() == 1 => {
                assert!(
                    t.trim_ascii().is_empty(),
                    "content outside the root element (not well-formed)"
                );
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert_eq!(stack.len(), 1, "unclosed element at end of document");
    let mut doc = stack.pop().expect("document node");
    assert_eq!(doc.children.len(), 1, "exactly one root element");
    doc.children.pop().expect("root element")
}
