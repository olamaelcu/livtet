//! RIS rendering from [`CslItem`].

use crate::csl::{CslItem, CslName};

/// Render one item as an RIS reference block.
///
/// Field order is fixed: `TY`, authors (`AU`), editors (`A2`), translators
/// (`A3`), title (`TI`), publisher (`PB`), place (`CY`), year (`PY`),
/// edition (`ET`), language (`LA`), ISBN (`SN`), DOI (`DO`), OCLC and LCCN
/// notes (`N1`), terminated by `ER  -`.
pub fn to_ris(item: &CslItem) -> String {
    let mut block = String::new();
    line(&mut block, "TY", "BOOK");

    render_names(&mut block, "AU", &item.author);
    render_names(&mut block, "A2", &item.editor);
    render_names(&mut block, "A3", &item.translator);

    line_optional(&mut block, "TI", item.title.as_deref());
    line_optional(&mut block, "PB", item.publisher.as_deref());
    line_optional(&mut block, "CY", item.publisher_place.as_deref());
    line(&mut block, "PY", &item.year_label());
    line_optional(&mut block, "ET", item.edition.as_deref());
    line_optional(&mut block, "LA", item.language.as_deref());
    line_optional(&mut block, "SN", item.isbn.as_deref());
    line_optional(&mut block, "DO", item.doi.as_deref());

    if let Some(oclc) = &item.oclc {
        line(&mut block, "N1", &format!("OCLC: {oclc}"));
    }
    if let Some(lccn) = &item.lccn {
        line(&mut block, "N1", &format!("LCCN: {lccn}"));
    }

    block.push_str("ER  -\n");
    block
}

fn render_names(block: &mut String, tag: &str, names: &[CslName]) {
    for name in names.iter().filter_map(CslName::display_name) {
        line(block, tag, &name);
    }
}

fn line_optional(block: &mut String, tag: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        line(block, tag, value);
    }
}

fn line(block: &mut String, tag: &str, value: &str) {
    block.push_str(tag);
    block.push_str("  - ");
    block.push_str(value);
    block.push('\n');
}
