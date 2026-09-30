//! `[copilot]`: what one Copilot AI credit costs this factory.

use serde::Deserialize;

use crate::cost::USD_PER_COPILOT_CREDIT;

/// How Copilot CLI runs are priced.
///
/// Copilot CLI reports the AI credits a run used, not dollars. Layover prices them at
/// `usd_per_credit`, which defaults to the published rate of a cent a credit; a factory billed at a
/// different rate says so here, and every Copilot run is priced with it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopilotConfig {
    /// What one AI credit costs, in US dollars.
    #[serde(default = "default_usd_per_credit")]
    pub usd_per_credit: f64,
}

impl Default for CopilotConfig {
    fn default() -> Self {
        Self {
            usd_per_credit: default_usd_per_credit(),
        }
    }
}

fn default_usd_per_credit() -> f64 {
    USD_PER_COPILOT_CREDIT
}
