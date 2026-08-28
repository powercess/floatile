//! UI theme token registry（guest-safe 单一语义源）。

/// P0 固定主题 token；值由宿主 renderer 消费，插件只持有稳定名称。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColorToken {
    pub name: &'static str,
    pub value: &'static str,
}

pub const COLOR_TOKENS: &[ThemeColorToken] = &[
    ThemeColorToken {
        name: "foreground",
        value: "#cdd6f4",
    },
    ThemeColorToken {
        name: "muted",
        value: "#9399b2",
    },
    ThemeColorToken {
        name: "accent",
        value: "#89b4fa",
    },
    ThemeColorToken {
        name: "positive",
        value: "#a6e3a1",
    },
    ThemeColorToken {
        name: "warning",
        value: "#f9e2af",
    },
    ThemeColorToken {
        name: "danger",
        value: "#f38ba8",
    },
];

pub fn color_token(name: &str) -> Option<&'static ThemeColorToken> {
    COLOR_TOKENS.iter().find(|token| token.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_names_and_values_are_bounded_literals() {
        for token in COLOR_TOKENS {
            assert!(!token.name.is_empty());
            assert_eq!(token.value.len(), 7);
            assert!(token.value.starts_with('#'));
            assert!(token.value[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
        }
    }
}
