//! The words the customer app and the embed show, per offered language (`config/copy/<tag>.yaml`).
//! Copy is config, never code: the customer's language setting picks the pack, and an insurer
//! replaces the illustrative wording with its approved wording. Checked at start-up: every
//! offered language has a pack with exactly the default pack's keys, each with the same
//! `{placeholders}`, so no screen is ever half translated.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(untagged)]
enum Node {
    Text(String),
    Group(BTreeMap<String, Node>),
}

fn flatten(prefix: &str, node: Node, out: &mut BTreeMap<String, String>) {
    match node {
        Node::Text(t) => {
            out.insert(prefix.to_string(), t);
        }
        Node::Group(g) => {
            for (k, v) in g {
                let key = if prefix.is_empty() {
                    k
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
    }
}

fn placeholders(text: &str) -> BTreeSet<&str> {
    text.split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .collect()
}

/// One language's copy, as the apps fetch it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CopyPack {
    pub language: String,
    /// Text by dotted key, such as `plans.title`; `{name}` marks a value the app fills in.
    pub text: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct CopyPacks {
    default: String,
    packs: BTreeMap<String, CopyPack>,
}

impl CopyPacks {
    /// From `(tag, text)` files; `default` and `offered` are the insurer's languages.
    pub fn load(
        files: &[(String, String)],
        default: &str,
        offered: &BTreeSet<String>,
    ) -> Result<CopyPacks, String> {
        let mut packs = BTreeMap::new();
        for (tag, text) in files {
            let node: Node =
                serde_saphyr::from_str(text).map_err(|e| format!("copy/{tag}.yaml: {e}"))?;
            let mut flat = BTreeMap::new();
            flatten("", node, &mut flat);
            packs.insert(
                tag.clone(),
                CopyPack {
                    language: tag.clone(),
                    text: flat,
                },
            );
        }
        let base = packs
            .get(default)
            .ok_or_else(|| format!("copy/{default}.yaml is missing for the default language"))?
            .clone();
        for tag in offered {
            let pack = packs
                .get(tag)
                .ok_or_else(|| format!("copy/{tag}.yaml is missing for an offered language"))?;
            let keys: BTreeSet<&String> = pack.text.keys().collect();
            let want: BTreeSet<&String> = base.text.keys().collect();
            if keys != want {
                let diff: Vec<&&String> = keys.symmetric_difference(&want).collect();
                return Err(format!(
                    "copy/{tag}.yaml keys differ from {default}: {diff:?}"
                ));
            }
            for (key, text) in &pack.text {
                let base_text = base.text.get(key).map_or("", String::as_str);
                if placeholders(text) != placeholders(base_text) {
                    return Err(format!(
                        "copy/{tag}.yaml {key}: placeholders differ from {default}"
                    ));
                }
            }
        }
        packs.retain(|tag, _| offered.contains(tag));
        Ok(CopyPacks {
            default: default.to_string(),
            packs,
        })
    }

    /// The pack for a language setting: that language when offered, otherwise the default.
    pub fn pack(&self, setting: Option<&str>) -> Option<&CopyPack> {
        setting
            .and_then(|l| self.packs.get(l))
            .or_else(|| self.packs.get(&self.default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered() -> BTreeSet<String> {
        ["en".to_string(), "zh-Hant".to_string()].into()
    }

    #[test]
    fn every_offered_language_has_the_same_keys_and_placeholders() {
        let en = (
            "en".to_string(),
            "plans:\n  title: Pick your price\n  per_month: \"{price} a month\"\n".to_string(),
        );
        let zh = (
            "zh-Hant".to_string(),
            "plans:\n  title: 揀你的價錢\n  per_month: \"每月 {price}\"\n".to_string(),
        );
        let packs = CopyPacks::load(&[en.clone(), zh], "en", &offered()).expect("loads");
        assert_eq!(
            packs
                .pack(Some("zh-Hant"))
                .map(|p| p.text["plans.per_month"].as_str()),
            Some("每月 {price}")
        );
        assert_eq!(
            packs.pack(Some("fr")).map(|p| p.language.as_str()),
            Some("en")
        );

        let missing = (
            "zh-Hant".to_string(),
            "plans:\n  title: 揀你的價錢\n".to_string(),
        );
        assert!(CopyPacks::load(&[en.clone(), missing], "en", &offered()).is_err());
        let renamed = (
            "zh-Hant".to_string(),
            "plans:\n  title: 揀你的價錢\n  per_month: \"每月 {amount}\"\n".to_string(),
        );
        assert!(CopyPacks::load(&[en.clone(), renamed], "en", &offered()).is_err());
        assert!(CopyPacks::load(&[en], "en", &offered()).is_err());
    }
}
