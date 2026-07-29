use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsPaneContribution {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub tag: String,
    pub module: String,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookDetailSectionContribution {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub tag: String,
    pub module: String,
    pub priority: i32,
    #[serde(default)]
    pub default_collapsed: bool,
    #[serde(default = "default_position")]
    pub position: String,
}

fn default_position() -> String {
    "after-metadata".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultEnhancementContribution {
    pub id: String,
    pub label: String,
    pub tag: String,
    pub module: String,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebContributions {
    #[serde(default)]
    pub settings_panes: Vec<SettingsPaneContribution>,
    #[serde(default)]
    pub book_detail_sections: Vec<BookDetailSectionContribution>,
    #[serde(default)]
    pub search_result_enhancements: Vec<SearchResultEnhancementContribution>,
}

/// Serializable shape for Tauri IPC.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SlotContribution {
    pub plugin_id: String,
    pub contribution_id: String,
    pub title: String,
    pub tag: String,
    pub module: String,
    pub priority: i32,
}

pub struct WebContributionRegistry {
    by_slot: HashMap<String, Vec<SlotContribution>>,
}

impl Default for WebContributionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WebContributionRegistry {
    pub fn new() -> Self {
        Self {
            by_slot: HashMap::new(),
        }
    }

    /// Register a single plugin's web contributions.
    pub fn register(&mut self, plugin_id: &str, contributions: &WebContributions) {
        for sc in &contributions.settings_panes {
            self.by_slot
                .entry("settings-panes".to_string())
                .or_default()
                .push(SlotContribution {
                    plugin_id: plugin_id.to_string(),
                    contribution_id: sc.id.clone(),
                    title: sc.title.clone(),
                    tag: sc.tag.clone(),
                    module: sc.module.clone(),
                    priority: sc.priority,
                });
        }
        for bc in &contributions.book_detail_sections {
            self.by_slot
                .entry("book-detail-sections".to_string())
                .or_default()
                .push(SlotContribution {
                    plugin_id: plugin_id.to_string(),
                    contribution_id: bc.id.clone(),
                    title: bc.title.clone(),
                    tag: bc.tag.clone(),
                    module: bc.module.clone(),
                    priority: bc.priority,
                });
        }
        for ec in &contributions.search_result_enhancements {
            self.by_slot
                .entry("search-result-enhancements".to_string())
                .or_default()
                .push(SlotContribution {
                    plugin_id: plugin_id.to_string(),
                    contribution_id: ec.id.clone(),
                    title: ec.label.clone(),
                    tag: ec.tag.clone(),
                    module: ec.module.clone(),
                    priority: ec.priority,
                });
        }
    }

    /// Get all contributions for a slot, sorted by (priority, plugin_id).
    pub fn get(&self, slot_type: &str) -> Vec<&SlotContribution> {
        let mut items: Vec<&SlotContribution> =
            self.by_slot.get(slot_type).into_iter().flatten().collect();
        items.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then(a.plugin_id.cmp(&b.plugin_id))
        });
        items
    }
}

pub fn parse_web_contributions(web: &serde_json::Value) -> WebContributions {
    serde_json::from_value(web.clone()).unwrap_or_default()
}
