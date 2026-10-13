//! #1146: dropping a hand-built, very deep tree must not recurse (the compiler
//! drop glue overflowed the stack), and hostile deeply nested XML must come
//! back as `Err`, never a stack overflow.

use std::thread;
use ttml_subtitle::document::{EbuttmElement, MetadataChild, MetadataElement};
use ttml_subtitle::{Document, InlineContent, SpanElement, UnknownElement, UnknownNode};

const DEPTH: usize = 1_000_000;
/// Far smaller than the main-thread default: one recursive drop frame per level
/// would need hundreds of MB.
const SMALL_STACK: usize = 256 * 1024;
/// Parsing legitimately recurses to `MAX_NESTING_DEPTH` (64) in a debug build,
/// so it gets 2 MiB; 200 000 unbounded levels would need orders of magnitude more.
const PARSE_STACK: usize = 2 * 1024 * 1024;

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    on_stack(SMALL_STACK, f)
}

fn on_stack(size: usize, f: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(size)
        .spawn(f)
        .unwrap()
        .join()
        .expect("thread panicked");
}

#[test]
fn drop_of_million_deep_span_chain_does_not_overflow() {
    on_small_stack(|| {
        let mut cur = SpanElement::default();
        for _ in 0..DEPTH {
            let mut parent = SpanElement::default();
            parent.content.push(InlineContent::Text("x".into()));
            parent.content.push(InlineContent::Span(Box::new(cur)));
            cur = parent;
        }
        drop(cur);
    });
}

#[test]
fn drop_of_million_deep_metadata_chain_does_not_overflow() {
    on_small_stack(|| {
        let mut cur = MetadataElement::default();
        for i in 0..DEPTH {
            // Alternate <metadata> and <ebuttm:documentMetadata> levels.
            if i % 2 == 0 {
                let mut parent = MetadataElement::default();
                parent.children.push(MetadataChild::Metadata(cur));
                cur = parent;
            } else {
                let mut mid = EbuttmElement::default();
                mid.children.push(MetadataChild::Metadata(cur));
                let mut parent = MetadataElement::default();
                parent
                    .children
                    .push(MetadataChild::EbuttmDocumentMetadata(mid));
                cur = parent;
            }
        }
        drop(cur);
    });
}

#[test]
fn drop_of_million_deep_unknown_chain_does_not_overflow() {
    on_small_stack(|| {
        let mut cur = UnknownElement::new(None, None, "x");
        for _ in 0..DEPTH {
            let mut parent = UnknownElement::new(None, None, "x");
            parent.children.push(UnknownNode::Text("t".into()));
            parent.children.push(UnknownNode::Element(Box::new(cur)));
            cur = parent;
        }
        drop(cur);
    });
}

/// Hostile XML nested far beyond any limit, for every recursive element kind,
/// is an `Err` on a tiny stack.
#[test]
fn hostile_deep_xml_is_err_not_overflow() {
    on_stack(PARSE_STACK, || {
        let depth = 200_000;
        let wrap = |open: &str, close: &str| {
            format!(
                "<tt xmlns=\"http://www.w3.org/ns/ttml\" \
                 xmlns:ttm=\"http://www.w3.org/ns/ttml#metadata\" \
                 xmlns:x=\"urn:x\"><head>{}{}</head></tt>",
                open.repeat(depth),
                close.repeat(depth)
            )
        };
        let cases = [
            wrap("<metadata>", "</metadata>"),
            wrap("<x:u>", "</x:u>"),
            wrap("<metadata><ttm:agent>", "</ttm:agent></metadata>"),
        ];
        for xml in cases {
            assert!(Document::parse_str(&xml).is_err(), "deep XML accepted");
        }
        let spans = format!(
            "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div><p>{}{}</p></div></body></tt>",
            "<span>".repeat(depth),
            "</span>".repeat(depth)
        );
        assert!(Document::parse_str(&spans).is_err());
    });
}
