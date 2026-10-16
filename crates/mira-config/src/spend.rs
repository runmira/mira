//! Spend policy is global-only: a checked-out project cannot raise a cap.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpendCaps {
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "amount")]
    pub per_session: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "amount")]
    pub per_day: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "amount")]
    pub per_month: Option<f64>,
}

fn amount<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let value = Option::<f64>::deserialize(d)?;
    if value.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err(serde::de::Error::custom(
            "spend limits must be finite and greater than zero",
        ));
    }
    Ok(value)
}

impl SpendCaps {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpendLimits {
    #[serde(flatten)]
    pub caps: SpendCaps,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, SpendCaps>,
}

impl SpendLimits {
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty() && self.projects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_round_trips_caps() {
        for v in ["0", "-1", ".nan", ".inf"] {
            assert!(serde_yaml::from_str::<SpendCaps>(&format!("per_day: {v}")).is_err());
        }
        let s = "per_session: 5\nper_day: 20\nprojects:\n  /repo:\n    per_month: 100\n";
        let c: SpendLimits = serde_yaml::from_str(s).unwrap();
        assert_eq!(c.caps.per_session, Some(5.0));
        assert_eq!(c.projects["/repo"].per_month, Some(100.0));
        let back: SpendLimits = serde_yaml::from_str(&serde_yaml::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
    #[test]
    fn project_files_cannot_change_user_policy() {
        let local: crate::MiraConfig =
            serde_yaml::from_str("spend_limits:\n  per_day: 999\n").unwrap();
        assert!(crate::MiraConfig::default()
            .merge(local)
            .spend_limits
            .is_empty());
    }
}
