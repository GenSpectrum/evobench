//! Should add these to ahtml crate.

use ahtml::{AId, AllocatorType, HtmlAllocator, Node, ToASlice, att, flat::Flat};
use anyhow::Result;

// Utils for `Flat`

/// How many items the Flat contains.
pub fn flat_len<T: AllocatorType>(slf: &Flat<T>) -> u32 {
    match slf {
        Flat::None => 0,
        Flat::One(_) => 1,
        Flat::Two(_, _) => 2,
        Flat::Slice(s) => s.len(),
    }
}

pub fn flat_get<'t>(slf: &Flat<Node>, index: u32, html: &'t HtmlAllocator) -> Option<&'t Node> {
    match slf {
        Flat::None => None,
        Flat::One(v) => {
            if index == 0 {
                html.get_node(*v)
            } else {
                None
            }
        }
        Flat::Two(a, b) => {
            let v = match index {
                0 => a,
                1 => b,
                _ => return None,
            };
            html.get_node(*v)
        }
        Flat::Slice(s) => html.get_node(s.get(index, html)?),
    }
}

/// Strip outer `div` and `p` HTML elements and return the body of the
/// inner-most of them. E.g. `<div><p>Some <b>text</b>.</p></div>`
/// becomes `Some <b>text</b>.`. If not possible, returns the original
/// node. If `node` is not in `html`, silently returns the original
/// node. If `keep_if_attributes` is true, only strips elements when
/// they have no attributes.
pub fn extract_paragraph_body(
    node: AId<Node>,
    keep_if_attributes: bool,
    html: &HtmlAllocator,
) -> Flat<Node> {
    let mut body = Flat::One(node);
    loop {
        // If `body` is just 1 item, that is an element, and the
        // element is "div" or "p", unwrap its body.
        if flat_len(&body) == 1 {
            let node = flat_get(&body, 0, html).expect("checked len is 1");
            if let Some(element) = node.as_element() {
                match element.meta.tag_name.as_str() {
                    "div" | "p" => {
                        if (!keep_if_attributes) || element.attr.len() == 0 {
                            body = Flat::Slice(element.body)
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }
    body
}

/// Make an anchor (target for `#foo` style links) in the best compatible way
pub fn anchor(
    anchor_name: &str,
    body: impl ToASlice<Node>,
    html: &HtmlAllocator,
) -> Result<AId<Node>> {
    html.a([att("name", anchor_name), att("id", anchor_name)], body)
}
