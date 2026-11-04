//! BibTeX rendering from [`CslItem`].

use crate::csl::{CslItem, CslName};
use crate::key::{citation_key, deconflicted_keys};

/// Render one item as a BibTeX `@book` entry.
///
/// The key is [`citation_key`]; use [`to_bibtex_batch`] when several entries
/// may share a base key.
pub fn to_bibtex(item: &CslItem) -> String {
    to_bibtex_with_key(item, &citation_key(item))
}

/// Render a batch as BibTeX entries separated by a blank line.
///
/// Base keys that collide within the batch gain a stable id-derived suffix so
/// every entry in the returned text has a unique citation key.
pub fn to_bibtex_batch(items: &[CslItem]) -> String {
    let keys = deconflicted_keys(items);
    let mut rendered = String::new();
    for (index, (item, key)) in items.iter().zip(keys.iter()).enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&to_bibtex_with_key(item, key));
    }
    rendered
}

fn to_bibtex_with_key(item: &CslItem, key: &str) -> String {
    let mut entry = String::new();
    entry.push_str("@book{");
    entry.push_str(key);
    entry.push_str(",\n");

    render_names(&mut entry, "author", &item.author);
    render_names(&mut entry, "editor", &item.editor);
    render_names(&mut entry, "translator", &item.translator);

    if let Some(title) = &item.title {
        let escaped = bibtex_escape(title);
        if !escaped.is_empty() {
            entry.push_str("  title = {{");
            entry.push_str(&escaped);
            entry.push_str("}},\n");
        }
    }

    render_optional(&mut entry, "publisher", item.publisher.as_deref());
    render_optional(&mut entry, "address", item.publisher_place.as_deref());
    let year = item.year_label();
    render_optional(&mut entry, "year", Some(year.as_str()));
    render_optional(&mut entry, "edition", item.edition.as_deref());
    render_optional(&mut entry, "language", item.language.as_deref());
    render_optional(&mut entry, "isbn", item.isbn.as_deref());
    render_optional(&mut entry, "doi", item.doi.as_deref());
    render_optional(&mut entry, "oclc", item.oclc.as_deref());
    render_optional(&mut entry, "lccn", item.lccn.as_deref());
    render_optional(&mut entry, "pages", item.number_of_pages.as_deref());

    entry.push_str("}\n");
    entry
}

fn render_names(entry: &mut String, field: &str, names: &[CslName]) {
    let joined = names
        .iter()
        .filter_map(CslName::display_name)
        .map(|name| bibtex_escape(&name))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(" and ");
    render_optional(entry, field, Some(joined.as_str()));
}

fn render_optional(entry: &mut String, field: &str, value: Option<&str>) {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return;
    };
    entry.push_str("  ");
    entry.push_str(field);
    entry.push_str(" = {");
    entry.push_str(value);
    entry.push_str("},\n");
}

/// Escape the BibTeX-special characters. The backslash is handled first so
/// the escapes introduced for other characters are not escaped again.
fn bibtex_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '&' => escaped.push_str("\\&"),
            '%' => escaped.push_str("\\%"),
            '$' => escaped.push_str("\\$"),
            '#' => escaped.push_str("\\#"),
            '_' => escaped.push_str("\\_"),
            '~' => escaped.push_str("\\~"),
            '^' => escaped.push_str("\\^"),
            _ => escaped.push(character),
        }
    }
    escaped
}
