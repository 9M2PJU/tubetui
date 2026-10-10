//! Interface language. Every text is written where it is used, in both
//! languages: `lang.pick("Buscar", "Search")`, or `Text` in static tables.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Es,
    En,
}

impl Lang {
    pub fn next(self) -> Self {
        match self {
            Self::Es => Self::En,
            Self::En => Self::Es,
        }
    }

    /// Value as written in `config.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Es => "es",
            Self::En => "en",
        }
    }

    /// The language's own name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Es => "Español",
            Self::En => "English",
        }
    }

    /// `es` in Spanish, `en` in English.
    pub fn pick<T>(self, es: T, en: T) -> T {
        match self {
            Self::Es => es,
            Self::En => en,
        }
    }

    /// Spanish for `es*` locales (`es_AR.UTF-8`), English for everything else.
    pub fn from_locale(locale: &str) -> Self {
        if locale.starts_with("es") { Self::Es } else { Self::En }
    }
}

/// A text in both languages, for static tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub es: &'static str,
    pub en: &'static str,
}

impl Text {
    pub const fn new(es: &'static str, en: &'static str) -> Self {
        Self { es, en }
    }

    pub fn get(self, lang: Lang) -> &'static str {
        lang.pick(self.es, self.en)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_picks_spanish_only_for_spanish() {
        assert_eq!(Lang::from_locale("es_AR.UTF-8"), Lang::Es);
        assert_eq!(Lang::from_locale("es"), Lang::Es);
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(Lang::from_locale("C.UTF-8"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }

    #[test]
    fn texts_follow_the_language() {
        const T: Text = Text::new("salir", "quit");
        assert_eq!(T.get(Lang::Es), "salir");
        assert_eq!(T.get(Lang::En), "quit");
        assert_eq!(Lang::Es.next(), Lang::En);
        assert_eq!(Lang::En.next().as_str(), "es");
    }
}
