//! A tiny key/value configuration store.

use std::collections::HashMap;

pub struct Config {
    values: HashMap<String, String>,
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let values = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        Self { values }
    }

    /// The value for `key`, if set.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The value for `key`, or `default`.
    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }
}

pub fn port(config: &Config) -> u16 {
    config.get("port").and_then(|p| p.parse().ok()).unwrap_or(8080)
}
