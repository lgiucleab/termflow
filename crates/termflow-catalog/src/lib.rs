//! Model catalog: context window, max output, capabilities and prices per `provider/model`,
//! from models.dev (`api.json`), cached by the caller, with user overrides (`BUILD_PROMPT` §7.9).

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use termflow_types::Usage;

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";
pub const MODELS_DEV: &str = "models.dev";

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

/// What we know about a model. `None` = unknown, never a guess.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelCaps {
    pub context: Option<u32>,
    pub max_output: Option<u32>,
    pub tools: Option<bool>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub price: Option<Price>,
}

/// `[catalog.overrides."provider/model"]` in config: any field replaces the catalog's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Override {
    pub context: Option<u32>,
    pub max_output: Option<u32>,
    pub tools: Option<bool>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub price: Option<Price>,
}

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    models: HashMap<String, ModelCaps>,
}

#[derive(Debug, thiserror::Error)]
#[error("catalog: {0}")]
pub struct CatalogError(#[from] serde_json::Error);

#[derive(Deserialize)]
struct MdProvider {
    #[serde(default)]
    models: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MdModel {
    tool_call: Option<bool>,
    reasoning: Option<bool>,
    modalities: Option<MdModalities>,
    limit: Option<MdLimit>,
    cost: Option<Price>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MdModalities {
    input: Vec<String>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MdLimit {
    context: Option<u32>,
    output: Option<u32>,
}

fn key(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

impl Catalog {
    /// Parse models.dev `api.json`. Entries that don't parse are skipped, never fatal.
    pub fn from_models_dev(json: &str) -> Result<Self, CatalogError> {
        let raw: HashMap<String, serde_json::Value> = serde_json::from_str(json)?;
        let mut models = HashMap::new();
        for (pid, pv) in raw {
            let Ok(p) = serde_json::from_value::<MdProvider>(pv) else {
                continue;
            };
            for (mid, mv) in p.models {
                let Ok(m) = serde_json::from_value::<MdModel>(mv) else {
                    continue;
                };
                models.insert(
                    key(&pid, &mid),
                    ModelCaps {
                        context: m.limit.as_ref().and_then(|l| l.context),
                        max_output: m.limit.as_ref().and_then(|l| l.output),
                        tools: m.tool_call,
                        vision: m.modalities.map(|x| x.input.iter().any(|i| i == "image")),
                        reasoning: m.reasoning,
                        price: m.cost,
                    },
                );
            }
        }
        Ok(Self { models })
    }

    pub fn apply_overrides<'a>(&mut self, overrides: impl IntoIterator<Item = (&'a String, &'a Override)>) {
        for (k, o) in overrides {
            let c = self.models.entry(k.clone()).or_default();
            c.context = o.context.or(c.context);
            c.max_output = o.max_output.or(c.max_output);
            c.tools = o.tools.or(c.tools);
            c.vision = o.vision.or(c.vision);
            c.reasoning = o.reasoning.or(c.reasoning);
            c.price = o.price.or(c.price);
        }
    }

    /// Caps for `provider/model`. A custom endpoint (`custom`) matches by model id across
    /// providers when the id is unique there.
    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelCaps> {
        if let Some(c) = self.models.get(&key(provider, model)) {
            return Some(c);
        }
        if provider != "custom" {
            return None;
        }
        let suffix = format!("/{model}");
        let mut hits = self.models.iter().filter(|(k, _)| k.ends_with(&suffix));
        match (hits.next(), hits.next()) {
            (Some((_, c)), None) => Some(c),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

/// Cost of one attempt in USD. `input_includes_cache`: OpenAI-style usage counts cached
/// tokens inside `input`; Anthropic reports them separately. Reasoning is billed as output
/// and already inside `output` for every wire we support.
pub fn cost(price: &Price, u: &Usage, input_includes_cache: bool) -> f64 {
    let cache_read = f64::from(u.cache_read);
    let cache_write = f64::from(u.cache_write);
    let mut input = f64::from(u.input);
    if input_includes_cache {
        input = (input - cache_read - cache_write).max(0.0);
    }
    (input * price.input
        + f64::from(u.output) * price.output
        + cache_read * price.cache_read.unwrap_or(price.input)
        + cache_write * price.cache_write.unwrap_or(price.input))
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/catalog/models_dev.json");

    #[test]
    fn parses_models_dev_and_skips_garbage() {
        let c = Catalog::from_models_dev(FIXTURE).unwrap();
        assert_eq!(c.len(), 4);
        let m = c.get("anthropic", "claude-test-model").unwrap();
        assert_eq!(
            (m.context, m.max_output, m.tools, m.vision),
            (Some(200_000), Some(64_000), Some(true), Some(true))
        );
        assert_eq!(m.price.unwrap().cache_write, Some(3.75));
        let t = c.get("openai", "tiny-no-tools").unwrap();
        assert_eq!((t.tools, t.vision, t.price), (Some(false), None, None));
        assert!(c.get("openai", "nope").is_none());
        assert!(Catalog::from_models_dev("not json").is_err());
    }

    #[test]
    fn custom_matches_unique_model_id_and_overrides_win() {
        let mut c = Catalog::from_models_dev(FIXTURE).unwrap();
        assert!(c.get("custom", "gpt-test-model").is_some());
        assert!(c.get("anthropic", "gpt-test-model").is_none());
        let mut o = HashMap::new();
        o.insert(
            "ollama/local-coder".to_owned(),
            Override {
                context: Some(32_000),
                tools: Some(true),
                ..Default::default()
            },
        );
        o.insert(
            "openai/gpt-test-model".to_owned(),
            Override {
                context: Some(1000),
                ..Default::default()
            },
        );
        c.apply_overrides(&o);
        assert_eq!(c.get("ollama", "local-coder").unwrap().context, Some(32_000));
        let g = c.get("openai", "gpt-test-model").unwrap();
        assert_eq!((g.context, g.max_output), (Some(1000), Some(128_000)));
    }

    #[test]
    fn cost_handles_both_cache_conventions() {
        let p = Price {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.3),
            cache_write: Some(3.75),
        };
        let u = Usage {
            input: 1000,
            output: 100,
            cache_read: 10_000,
            cache_write: 2000,
            reasoning: 50,
        };
        // Anthropic: input excludes cache.
        let a = cost(&p, &u, false);
        assert!((a - (3000.0 + 1500.0 + 3000.0 + 7500.0) / 1e6).abs() < 1e-12);
        // OpenAI: input 13 000 includes the 12 000 cached/written.
        let o = cost(&p, &Usage { input: 13_000, ..u }, true);
        assert!((o - a).abs() < 1e-12);
    }
}
