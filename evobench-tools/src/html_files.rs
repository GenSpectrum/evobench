use std::{path::Path, sync::Arc};

use ahtml::{HtmlAllocator, att};
use anyhow::Result;

use crate::ctx;

/// Create a HTML file that when loaded in the browser redirects to
/// `url`
pub fn write_redirect_html_file(path: &Path, url: &str) -> Result<()> {
    let html = HtmlAllocator::new(1000000, Arc::new("write_redirect_html_file"));
    let doc = html.html(
        [],
        [
            html.head(
                [],
                [html.meta(
                    [
                        att("http-equiv", "refresh"),
                        // XX escaping?
                        att("content", format!("0, url={url}")),
                    ],
                    [],
                )?],
            )?,
            html.body(
                [],
                [html.p(
                    [],
                    [
                        html.text("Redirecting to ")?,
                        html.a([att("href", url)], [html.text(url)?])?,
                    ],
                )?],
            )?,
        ],
    )?;
    let doc_string = html.to_html_string(doc, true);
    std::fs::write(path, &doc_string).map_err(ctx!("writing html file to {path:?}"))?;
    Ok(())
}
