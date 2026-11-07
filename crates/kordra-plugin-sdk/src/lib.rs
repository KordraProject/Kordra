//! Serialized plugin manifest and permission vocabulary, protocol draft v1.
//! A sandboxed loader is NOT yet implemented.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<Capability>,
    pub permissions: Vec<Permission>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    DownloadEngine, Extractor, MediaProcessor, Automation,
    UiWidget, NotificationSink, CloudProvider,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Permission {
    Network { allow_hosts: Vec<String> },
    FileRead { sandbox_root: String },
    FileWrite { sandbox_root: String },
    Secrets { keys: Vec<String> },
    Events { names: Vec<String> },
}
impl PluginManifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_empty() || self.id.len() > 128 ||
            !self.id.chars().all(|c| c.is_ascii_alphanumeric() || ['.', '-', '_'].contains(&c)) {
            return Err("invalid plugin id");
        }
        if self.version.trim().is_empty() || self.api_version.trim().is_empty() { return Err("missing version"); }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_ids_are_rejected() {
        assert!(PluginManifest { id: "../bad".into(), version: "1".into(), api_version: "1".into(),
            capabilities: vec![], permissions: vec![] }.validate().is_err());
    }
}
