//! Per-conversation translation model and reasoning effort choices.

use indexmap::IndexMap;
use serde_json::Value;

use crate::core::catalog::reasoning_efforts;
use crate::core::text::{first_text, strip};

pub const DEFAULT_MODEL: &str = "默认模型";
pub const DEFAULT_EFFORT: &str = "默认";
pub const COMMON_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug)]
pub struct ModelChoice {
    pub label: String,
    pub slug: String,
}

/// The model field holds either a catalog label or a typed model ID.
#[derive(Clone, Debug)]
pub struct ModelChoices {
    pub model: String,
    pub effort: String,
    choices: Vec<ModelChoice>,
    records: IndexMap<String, Value>,
}

impl ModelChoices {
    pub fn new(model: &str, effort: &str) -> Self {
        ModelChoices {
            model: if model.is_empty() { DEFAULT_MODEL.into() } else { model.into() },
            effort: if effort.is_empty() { DEFAULT_EFFORT.into() } else { effort.into() },
            choices: vec![ModelChoice { label: DEFAULT_MODEL.into(), slug: String::new() }],
            records: IndexMap::new(),
        }
    }

    pub fn choices(&self) -> &[ModelChoice] {
        &self.choices
    }

    fn slug_of(&self, text: &str) -> String {
        let text = strip(text);
        self.choices.iter().find(|c| c.label == text).map_or_else(|| text.to_string(), |c| c.slug.clone())
    }

    /// The model ID passed to Codex; empty keeps Codex's own default.
    pub fn slug(&self) -> String {
        self.slug_of(&self.model)
    }

    /// The effort passed to Codex; empty keeps Codex's own default.
    pub fn effort_value(&self) -> String {
        if self.effort == DEFAULT_EFFORT { String::new() } else { strip(&self.effort).to_string() }
    }

    pub fn tooltip(&self) -> String {
        let slug = self.slug();
        if !slug.is_empty() && slug != self.model {
            format!("翻译模型 · {}\n{slug}", self.model)
        } else {
            format!("翻译模型 · {}", self.model)
        }
    }

    /// Replaces the catalog, keeping the selected model by ID.
    pub fn apply(&mut self, records: &[Value]) -> Option<String> {
        let selected = self.slug();
        self.records.clear();
        for model in records {
            if let Some(slug) = first_text([model.get("model"), model.get("id")]) {
                self.records.insert(slug, model.clone());
            }
        }
        let name =
            |slug: &str, model: &Value| first_text([model.get("displayName")]).unwrap_or_else(|| slug.to_string());
        let names: Vec<String> = self.records.iter().map(|(s, m)| name(s, m)).collect();
        self.choices.truncate(1);
        for (slug, model) in &self.records {
            let display = name(slug, model);
            let duplicate = names.iter().filter(|n| **n == display).count() > 1;
            let label = if duplicate || display == DEFAULT_MODEL { format!("{display}  [{slug}]") } else { display };
            self.choices.push(ModelChoice { label, slug: slug.clone() });
        }
        if let Some(choice) = self.choices.iter().find(|c| c.slug == selected) {
            self.model = choice.label.clone();
        } else {
            self.model = if selected.is_empty() { DEFAULT_MODEL.into() } else { selected };
        }
        self.update_efforts()
    }

    pub fn set_model(&mut self, text: &str) -> Option<String> {
        self.model = if strip(text).is_empty() { DEFAULT_MODEL.into() } else { text.to_string() };
        self.update_efforts()
    }

    /// The effort options for the current model. Advertised levels are used
    /// as-is; unknown models get common levels plus the current one.
    pub fn efforts(&self) -> Vec<String> {
        let slug = self.slug();
        let mut choices: Vec<String> = match reasoning_efforts(self.records.get(&slug)) {
            Some(supported) => supported,
            None => {
                let mut common: Vec<String> = COMMON_EFFORTS.map(String::from).to_vec();
                if self.effort != DEFAULT_EFFORT && !common.contains(&self.effort) {
                    common.push(self.effort.clone());
                }
                common
            }
        };
        choices.insert(0, DEFAULT_EFFORT.into());
        choices
    }

    /// Resets an effort the selected model does not support. Returns a log line.
    fn update_efforts(&mut self) -> Option<String> {
        let slug = self.slug();
        let supported = reasoning_efforts(self.records.get(&slug))?;
        if self.effort != DEFAULT_EFFORT && !supported.contains(&self.effort) {
            let message = format!("{slug} 不支持 {}，Effort 已恢复默认。", self.effort);
            self.effort = DEFAULT_EFFORT.into();
            return Some(message);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn records() -> Vec<Value> {
        vec![
            json!({"model": "m1", "displayName": "New label", "supportedReasoningEfforts": [
                {"reasoningEffort": "high"}, {"reasoningEffort": "max"}]}),
            json!({"model": "m2", "displayName": "Small", "supportedReasoningEfforts": [{"reasoningEffort": "low"}]}),
            json!({"model": "m3", "displayName": "Small"}),
        ]
    }

    #[test]
    fn catalog_keeps_selection_by_id_and_disambiguates_names() {
        let mut choices = ModelChoices::new("m1", "max");
        assert_eq!(choices.apply(&records()), None);
        assert_eq!((choices.model.as_str(), choices.slug().as_str()), ("New label", "m1"));
        assert_eq!(choices.effort_value(), "max");
        assert_eq!(choices.efforts(), [DEFAULT_EFFORT, "high", "max"]);
        let labels: Vec<&str> = choices.choices().iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, [DEFAULT_MODEL, "New label", "Small  [m2]", "Small  [m3]"]);
    }

    #[test]
    fn unsupported_effort_resets_to_default() {
        let mut choices = ModelChoices::new("", "max");
        choices.apply(&records());
        let message = choices.set_model("Small  [m2]").unwrap();
        assert!(message.contains("m2 不支持 max"));
        assert_eq!((choices.effort.as_str(), choices.effort_value().as_str()), (DEFAULT_EFFORT, ""));
    }

    #[test]
    fn typed_model_and_unknown_catalog_keep_custom_effort() {
        let mut choices = ModelChoices::new("custom-model", "future-level");
        assert_eq!(choices.slug(), "custom-model");
        assert_eq!(choices.efforts().last().unwrap(), "future-level");
        choices.set_model("  ");
        assert_eq!((choices.model.as_str(), choices.slug().as_str()), (DEFAULT_MODEL, ""));
    }
}
