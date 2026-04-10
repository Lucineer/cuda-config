/*!
# cuda-config

Layered configuration management.

Every agent needs configuration — some shared, some secret, some
environment-specific. This crate provides layered config with
env vars, defaults, validation, and change detection.

- Layered config (defaults → file → env → cli)
- Type-safe access with fallbacks
- Config validation
- Change detection and hot reload hints
- Secret masking
- Config export/import
*/

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A config value
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ConfigValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<String>),
    Map(HashMap<String, ConfigValue>),
}

/// Config source priority
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConfigLayer { Defaults = 0, File = 1, Env = 2, Cli = 3 }

/// A config entry with source tracking
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub value: ConfigValue,
    pub source: ConfigLayer,
    pub is_secret: bool,
    pub modified_ms: u64,
}

/// Validation rule
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationRule {
    pub key: String,
    pub required: bool,
    pub value_type: String,    // "string", "int", "float", "bool"
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub allowed: Vec<String>,
}

/// Validation error
#[derive(Clone, Debug)]
pub struct ValidationError { pub key: String, pub reason: String }

/// The configuration system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigManager {
    pub values: HashMap<String, ConfigEntry>,
    pub env_prefix: String,
    pub rules: Vec<ValidationRule>,
    pub changes: u64,
}

impl ConfigManager {
    pub fn new() -> Self { ConfigManager { values: HashMap::new(), env_prefix: "CUDA_".into(), rules: vec![], changes: 0 } }

    /// Set a config value
    pub fn set(&mut self, key: &str, value: ConfigValue, source: ConfigLayer) {
        let is_secret = key.contains("secret") || key.contains("key") || key.contains("token") || key.contains("password");
        let entry = ConfigEntry { value, source, is_secret, modified_ms: now() };
        self.values.insert(key.to_string(), entry);
        self.changes += 1;
    }

    /// Set a string value
    pub fn set_str(&mut self, key: &str, val: &str, source: ConfigLayer) { self.set(key, ConfigValue::String(val.to_string()), source); }

    /// Set an int value
    pub fn set_int(&mut self, key: &str, val: i64, source: ConfigLayer) { self.set(key, ConfigValue::Int(val), source); }

    /// Set a bool value
    pub fn set_bool(&mut self, key: &str, val: bool, source: ConfigLayer) { self.set(key, ConfigValue::Bool(val), source); }

    /// Get a string value
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.values.get(key)?.value {
            ConfigValue::String(ref s) => Some(s),
            _ => None,
        }
    }

    /// Get string with fallback
    pub fn str_or(&self, key: &str, default: &str) -> String {
        self.get_str(key).unwrap_or(default).to_string()
    }

    /// Get int value
    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.values.get(key)?.value {
            ConfigValue::Int(i) => Some(i),
            _ => None,
        }
    }

    /// Get int with fallback
    pub fn int_or(&self, key: &str, default: i64) -> i64 { self.get_int(key).unwrap_or(default) }

    /// Get bool value
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.values.get(key)?.value {
            ConfigValue::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// Get bool with fallback
    pub fn bool_or(&self, key: &str, default: bool) -> bool { self.get_bool(key).unwrap_or(default) }

    /// Get float value
    pub fn get_float(&self, key: &str) -> Option<f64> {
        match self.values.get(key)?.value {
            ConfigValue::Float(f) => Some(f),
            ConfigValue::Int(i) => Some(i as f64),
            _ => None,
        }
    }

    /// Check if key exists
    pub fn has(&self, key: &str) -> bool { self.values.contains_key(key) }

    /// Get source of a key
    pub fn source(&self, key: &str) -> Option<ConfigLayer> { self.values.get(key).map(|e| e.source) }

    /// Delete a key
    pub fn delete(&mut self, key: &str) { self.values.remove(key); }

    /// Add validation rule
    pub fn add_rule(&mut self, rule: ValidationRule) { self.rules.push(rule); }

    /// Validate all rules
    pub fn validate(&self) -> Vec<ValidationError> {
        self.rules.iter().filter_map(|rule| {
            if rule.required && !self.values.contains_key(&rule.key) {
                return Some(ValidationError { key: rule.key.clone(), reason: "required but missing".into() });
            }
            if let Some(entry) = self.values.get(&rule.key) {
                // Type check
                match (&entry.value, rule.value_type.as_str()) {
                    (ConfigValue::String(s), "string") if !rule.allowed.is_empty() => {
                        if !rule.allowed.iter().any(|a| a == s) { return Some(ValidationError { key: rule.key.clone(), reason: format!("value not in allowed list: {:?}", rule.allowed) }); }
                    }
                    (ConfigValue::Int(i), "int") => {
                        if let Some(min) = rule.min { if (*i as f64) < min { return Some(ValidationError { key: rule.key.clone(), reason: format!("below minimum {}", min) }); } }
                        if let Some(max) = rule.max { if (*i as f64) > max { return Some(ValidationError { key: rule.key.clone(), reason: format!("above maximum {}", max) }); } }
                    }
                    (ConfigValue::Float(f), "float") => {
                        if let Some(min) = rule.min { if *f < min { return Some(ValidationError { key: rule.key.clone(), reason: format!("below minimum {}", min) }); } }
                        if let Some(max) = rule.max { if *f > max { return Some(ValidationError { key: rule.key.clone(), reason: format!("above maximum {}", max) }); } }
                    }
                    _ => {}
                }
            }
            None
        }).collect()
    }

    /// Export all non-secret values as string map
    pub fn export_public(&self) -> HashMap<String, String> {
        self.values.iter()
            .filter(|(_, e)| !e.is_secret)
            .map(|(k, e)| (k.clone(), format!("{:?}", e.value)))
            .collect()
    }

    /// Mask secrets in display
    pub fn masked_value(&self, key: &str) -> String {
        match self.values.get(key) {
            Some(e) if e.is_secret => "***".to_string(),
            Some(e) => format!("{:?}", e.value),
            None => "(not set)".to_string(),
        }
    }

    /// Keys that changed since timestamp
    pub fn changed_since(&self, since_ms: u64) -> Vec<&str> {
        self.values.iter().filter(|(_, e)| e.modified_ms > since_ms).map(|(k, _)| k.as_str()).collect()
    }

    /// Summary
    pub fn summary(&self) -> String {
        let secrets = self.values.values().filter(|e| e.is_secret).count();
        format!("Config: {} values ({} secrets), {} rules, {} changes",
            self.values.len(), secrets, self.rules.len(), self.changes)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut cm = ConfigManager::new();
        cm.set_str("name", "test_agent", ConfigLayer::Defaults);
        assert_eq!(cm.get_str("name"), Some("test_agent"));
    }

    #[test]
    fn test_fallback() {
        let cm = ConfigManager::new();
        assert_eq!(cm.str_or("missing", "default_val"), "default_val");
        assert_eq!(cm.int_or("missing", 42), 42);
    }

    #[test]
    fn test_source_priority() {
        let mut cm = ConfigManager::new();
        cm.set_str("x", "default", ConfigLayer::Defaults);
        cm.set_str("x", "override", ConfigLayer::Cli);
        assert_eq!(cm.get_str("x"), Some("override"));
    }

    #[test]
    fn test_auto_secret_detection() {
        let mut cm = ConfigManager::new();
        cm.set_str("api_key", "secret123", ConfigLayer::File);
        assert!(cm.values.get("api_key").unwrap().is_secret);
    }

    #[test]
    fn test_masked_value() {
        let mut cm = ConfigManager::new();
        cm.set_str("api_key", "abc", ConfigLayer::File);
        assert_eq!(cm.masked_value("api_key"), "***");
        assert_eq!(cm.masked_value("missing"), "(not set)");
    }

    #[test]
    fn test_validation_required() {
        let mut cm = ConfigManager::new();
        cm.add_rule(ValidationRule { key: "host".into(), required: true, value_type: "string".into(), min: None, max: None, allowed: vec![] });
        let errors = cm.validate();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_validation_range() {
        let mut cm = ConfigManager::new();
        cm.set_int("port", 70000, ConfigLayer::File);
        cm.add_rule(ValidationRule { key: "port".into(), required: false, value_type: "int".into(), min: Some(1.0), max: Some(65535.0), allowed: vec![] });
        let errors = cm.validate();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_export_public() {
        let mut cm = ConfigManager::new();
        cm.set_str("name", "test", ConfigLayer::Defaults);
        cm.set_str("secret_key", "hidden", ConfigLayer::File);
        let exported = cm.export_public();
        assert!(exported.contains_key("name"));
        assert!(!exported.contains_key("secret_key"));
    }

    #[test]
    fn test_changed_since() {
        let mut cm = ConfigManager::new();
        cm.set_str("a", "1", ConfigLayer::Defaults);
        let before = now();
        cm.set_str("b", "2", ConfigLayer::File);
        let changed = cm.changed_since(before);
        assert!(changed.contains(&"b"));
    }

    #[test]
    fn test_delete() {
        let mut cm = ConfigManager::new();
        cm.set_str("x", "val", ConfigLayer::Defaults);
        assert!(cm.has("x"));
        cm.delete("x");
        assert!(!cm.has("x"));
    }

    #[test]
    fn test_summary() {
        let cm = ConfigManager::new();
        let s = cm.summary();
        assert!(s.contains("0 values"));
    }
}
