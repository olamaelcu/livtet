use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ResolvedLink {
    pub label: String,
    pub url: String,
    pub category: LinkCategory,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,
    #[serde(default = "default_sort_hint")]
    pub sort_hint: i32,
    #[serde(default)]
    pub affiliate: bool,
}

fn default_sort_hint() -> i32 {
    100
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LinkCategory {
    Buy,
    Borrow,
    Reference,
    Social,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct ResolveLinksOptions {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<LinkCategory>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct ResolveLinksResult {
    pub links: Vec<ResolvedLink>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn link_category_deserializes_from_lowercase() {
        assert_eq!(
            serde_json::from_str::<LinkCategory>("\"buy\"").unwrap(),
            LinkCategory::Buy
        );
        assert_eq!(
            serde_json::from_str::<LinkCategory>("\"borrow\"").unwrap(),
            LinkCategory::Borrow
        );
        assert_eq!(
            serde_json::from_str::<LinkCategory>("\"reference\"").unwrap(),
            LinkCategory::Reference
        );
        assert_eq!(
            serde_json::from_str::<LinkCategory>("\"social\"").unwrap(),
            LinkCategory::Social
        );
    }

    #[test]
    fn link_category_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_value(LinkCategory::Buy).unwrap(),
            json!("buy")
        );
        assert_eq!(
            serde_json::to_value(LinkCategory::Reference).unwrap(),
            json!("reference")
        );
    }

    #[test]
    fn resolved_link_sort_hint_defaults_to_100() {
        let json = json!({
            "label": "Example",
            "url": "https://example.com",
            "category": "buy"
        });
        let link: ResolvedLink = serde_json::from_value(json).unwrap();
        assert_eq!(link.sort_hint, 100);
    }

    #[test]
    fn resolved_link_affiliate_defaults_to_false() {
        let json = json!({
            "label": "Example",
            "url": "https://example.com",
            "category": "buy"
        });
        let link: ResolvedLink = serde_json::from_value(json).unwrap();
        assert!(!link.affiliate);
    }

    #[test]
    fn resolved_link_icon_is_optional() {
        let json = json!({
            "label": "Example",
            "url": "https://example.com",
            "category": "buy",
            "icon": "amazon"
        });
        let link: ResolvedLink = serde_json::from_value(json).unwrap();
        assert_eq!(link.icon.as_deref(), Some("amazon"));

        let minimal = json!({
            "label": "Example",
            "url": "https://example.com",
            "category": "buy"
        });
        let minimal_link: ResolvedLink = serde_json::from_value(minimal).unwrap();
        assert!(minimal_link.icon.is_none());
    }

    #[test]
    fn resolved_link_roundtrip() {
        let original = ResolvedLink {
            label: "Buy now".to_string(),
            url: "https://example.com/buy".to_string(),
            category: LinkCategory::Buy,
            icon: Some("cart".to_string()),
            sort_hint: 50,
            affiliate: true,
        };
        let value = serde_json::to_value(&original).unwrap();
        let restored: ResolvedLink = serde_json::from_value(value).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn resolved_link_omits_none_icon_on_serialize() {
        let link = ResolvedLink {
            label: "X".to_string(),
            url: "https://x".to_string(),
            category: LinkCategory::Social,
            icon: None,
            sort_hint: 100,
            affiliate: false,
        };
        let value = serde_json::to_value(&link).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("icon"));
    }

    #[test]
    fn resolve_links_options_default() {
        let options = ResolveLinksOptions::default();
        assert!(options.locale.is_none());
        assert!(options.categories.is_none());
    }

    #[test]
    fn resolve_links_options_roundtrip() {
        let original = ResolveLinksOptions {
            locale: Some("en-US".to_string()),
            categories: Some(vec![LinkCategory::Buy, LinkCategory::Borrow]),
        };
        let value = serde_json::to_value(&original).unwrap();
        let restored: ResolveLinksOptions = serde_json::from_value(value).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn resolve_links_result_default_is_empty() {
        let result = ResolveLinksResult::default();
        assert!(result.links.is_empty());
    }

    #[test]
    fn resolve_links_result_roundtrip() {
        let result = ResolveLinksResult {
            links: vec![ResolvedLink {
                label: "Test".to_string(),
                url: "https://t".to_string(),
                category: LinkCategory::Reference,
                icon: None,
                sort_hint: 10,
                affiliate: false,
            }],
        };
        let value = serde_json::to_value(&result).unwrap();
        let restored: ResolveLinksResult = serde_json::from_value(value).unwrap();
        assert_eq!(restored, result);
    }
}
