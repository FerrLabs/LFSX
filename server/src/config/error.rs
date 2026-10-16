use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
#[error("{variable}={value:?} is not {expected}")]
pub struct ConfigError {
    variable: String,
    value: String,
    expected: &'static str,
}

impl ConfigError {
    pub(super) fn new(variable: impl Into<String>, value: &str, expected: &'static str) -> Self {
        Self {
            variable: variable.into(),
            value: value.to_owned(),
            expected,
        }
    }
}

pub(super) fn parse<T: FromStr>(
    variable: &'static str,
    value: Option<&str>,
    expected: &'static str,
) -> Result<Option<T>, ConfigError> {
    value
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            raw.parse()
                .map_err(|_| ConfigError::new(variable, raw, expected))
        })
        .transpose()
}
