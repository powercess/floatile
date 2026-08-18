//! 主题 token 服务（声明能力 theme:subscribe）。
//!
//! P0 提供固定 token 集合；订阅用于接收主题变化通知（动态主题在后续切片）。

use std::collections::{BTreeMap, BTreeSet};

use crate::errors::ThemeError;

/// P0 固定主题 token（名称 → 值）。
const DEFAULT_TOKENS: &[(&str, &str)] = &[
    ("color.background", "#1e1e2e"),
    ("color.foreground", "#cdd6f4"),
    ("color.accent", "#89b4fa"),
    ("font.monospace", "monospace"),
];

pub struct ThemeService {
    tokens: BTreeMap<String, String>,
    subscriptions: BTreeSet<u32>,
    next_subscription: u32,
}

impl Default for ThemeService {
    fn default() -> Self {
        Self {
            tokens: DEFAULT_TOKENS
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            subscriptions: BTreeSet::new(),
            next_subscription: 1,
        }
    }
}

impl ThemeService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_token(&self, name: &str) -> Result<Option<String>, ThemeError> {
        if !self.tokens.contains_key(name) {
            return Err(ThemeError::UnknownToken);
        }
        Ok(self.tokens.get(name).cloned())
    }

    pub fn subscribe(&mut self) -> Result<u32, ThemeError> {
        let id = self.next_subscription;
        self.next_subscription = self.next_subscription.wrapping_add(1).max(1);
        self.subscriptions.insert(id);
        Ok(id)
    }

    pub fn unsubscribe(&mut self, id: u32) -> Result<(), ThemeError> {
        if self.subscriptions.remove(&id) {
            Ok(())
        } else {
            Err(ThemeError::InvalidSubscription)
        }
    }
}
