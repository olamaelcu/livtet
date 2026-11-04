//! Citation-key generation and batch de-duplication.

use std::collections::{HashMap, HashSet};

use crate::csl::CslItem;

/// The undeduplicated BibTeX key for one item.
///
/// The key is the first author's family name (or literal name), ASCII-folded
/// and lowercased with all non-alphanumerics removed, followed by the issued
/// year. A missing year contributes `nd`.
///
/// When several items in a batch share this base, the batch renderer appends
/// a stable suffix derived from each item's `id`.
pub fn citation_key(item: &CslItem) -> String {
    let family = item
        .author
        .first()
        .and_then(|name| name.family.as_deref().or(name.literal.as_deref()))
        .map(fold_ascii)
        .unwrap_or_default();
    let year = item
        .issued_year()
        .map(|year| year.to_string())
        .unwrap_or_else(|| "nd".to_owned());
    format!("{family}{year}")
}

/// One key per item, appending `-<id-hash>` to every base key that occurs
/// more than once in the slice.
///
/// The hash is derived from the item `id`, so the same batch always produces
/// the same keys regardless of item order. Should two distinct ids share a
/// hash prefix, the later key gains a `-2`, `-3`, ... ordinal.
pub(crate) fn deconflicted_keys(items: &[CslItem]) -> Vec<String> {
    let bases: Vec<String> = items.iter().map(citation_key).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for base in &bases {
        *counts.entry(base.as_str()).or_default() += 1;
    }

    let mut used: HashSet<String> = HashSet::new();
    let mut keys = Vec::with_capacity(items.len());
    for (item, base) in items.iter().zip(&bases) {
        let stem = if counts[base.as_str()] > 1 {
            format!("{base}-{}", short_id_hash(&item.id))
        } else {
            base.as_str().to_owned()
        };
        let mut key = stem.clone();
        let mut ordinal = 2;
        while !used.insert(key.clone()) {
            key = format!("{stem}-{ordinal}");
            ordinal += 1;
        }
        keys.push(key);
    }
    keys
}

/// Six hex digits of FNV-1a over the id — stable across runs and platforms.
fn short_id_hash(id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:06x}").chars().take(6).collect()
}

/// Fold Latin diacritics to ASCII, lowercase, and drop everything that is
/// not an ASCII alphanumeric.
fn fold_ascii(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        for lower in character.to_lowercase() {
            if lower.is_ascii_alphanumeric() {
                folded.push(lower);
            } else if let Some(replacement) = fold_non_ascii(lower) {
                folded.push_str(replacement);
            }
        }
    }
    folded
}

fn fold_non_ascii(character: char) -> Option<&'static str> {
    let replacement = match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ğ' | 'ĝ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ß' => "s",
        'ţ' | 'ť' | 'ŧ' | 'þ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    };
    Some(replacement)
}
