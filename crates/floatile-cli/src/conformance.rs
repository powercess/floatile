//! Versioned SDK conformance kit exposed to language adapters and CI.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::OUTPUT_SCHEMA_VERSION;

pub const LIFECYCLE_SUITE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../conformance/sdk-lifecycle-v1.json"
));

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleSuite {
    pub schema_version: u32,
    pub engine_api_version: String,
    pub vectors: Vec<LifecycleVector>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleVector {
    pub id: String,
    pub callback: String,
    pub guest_error: String,
    pub message: Option<String>,
    pub expected_host_outcome: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConformanceReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub suite: &'static str,
    pub contract: LifecycleSuite,
    pub warnings: Vec<crate::CommandWarning>,
}

#[derive(Debug, Error)]
pub enum ConformanceError {
    #[error("内置 conformance JSON 无效")]
    InvalidJson,
    #[error("不支持的 conformance schemaVersion: {0}")]
    UnsupportedSchema(u32),
    #[error("conformance 向量无效: {0}")]
    InvalidVector(String),
}

impl ConformanceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "FCONF_JSON",
            Self::UnsupportedSchema(_) => "FCONF_SCHEMA_VERSION",
            Self::InvalidVector(_) => "FCONF_VECTOR",
        }
    }
}

pub fn lifecycle_report() -> Result<ConformanceReport, ConformanceError> {
    let contract: LifecycleSuite =
        serde_json::from_str(LIFECYCLE_SUITE_JSON).map_err(|_| ConformanceError::InvalidJson)?;
    validate(&contract)?;
    Ok(ConformanceReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        status: "ok",
        suite: "sdk-lifecycle-v1",
        contract,
        warnings: Vec::new(),
    })
}

fn validate(suite: &LifecycleSuite) -> Result<(), ConformanceError> {
    if suite.schema_version != 1 {
        return Err(ConformanceError::UnsupportedSchema(suite.schema_version));
    }
    let mut ids = BTreeSet::new();
    for vector in &suite.vectors {
        if !ids.insert(vector.id.as_str()) {
            return Err(ConformanceError::InvalidVector("vector id 重复".to_owned()));
        }
        if !matches!(vector.callback.as_str(), "start" | "event") {
            return Err(ConformanceError::InvalidVector(
                "callback 不受支持".to_owned(),
            ));
        }
        if !matches!(
            vector.guest_error.as_str(),
            "invalid-input" | "rejected" | "internal"
        ) {
            return Err(ConformanceError::InvalidVector(
                "guestError 不受支持".to_owned(),
            ));
        }
        if vector.expected_host_outcome != "rejected" {
            return Err(ConformanceError::InvalidVector(
                "expectedHostOutcome 不受支持".to_owned(),
            ));
        }
        let requires_message = vector.guest_error != "internal";
        if requires_message != vector.message.is_some() {
            return Err(ConformanceError::InvalidVector(
                "message 与 guestError 不匹配".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_lifecycle_suite_is_valid_and_versioned() -> Result<(), ConformanceError> {
        let report = lifecycle_report()?;
        assert_eq!(report.schema_version, OUTPUT_SCHEMA_VERSION);
        assert_eq!(report.suite, "sdk-lifecycle-v1");
        assert_eq!(report.contract.schema_version, 1);
        assert_eq!(report.contract.engine_api_version, "1.2.0");
        assert_eq!(report.contract.vectors.len(), 6);
        Ok(())
    }
}
