use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub tracking_time: TrackingTimeConfig,
}

#[derive(Debug, Clone)]
pub struct TrackingTimeConfig {
    pub auth: TrackingTimeAuth,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub enum TrackingTimeAuth {
    Token(String),
    Basic { email: String, password: String },
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let auth = if let Ok(token) = std::env::var("TRACKING_TIME_API_TOKEN") {
            TrackingTimeAuth::Token(token)
        } else {
            TrackingTimeAuth::Basic {
                email: std::env::var("TRACKING_TIME_EMAIL")
                    .context("TRACKING_TIME_EMAIL o TRACKING_TIME_API_TOKEN requerido")?,
                password: std::env::var("TRACKING_TIME_PASSWORD")
                    .context("TRACKING_TIME_PASSWORD requerido cuando no hay API token")?,
            }
        };
        Ok(Config {
            tracking_time: TrackingTimeConfig {
                auth,
                base_url: std::env::var("TRACKING_TIME_BASE_URL")
                    .unwrap_or_else(|_| "https://app.trackingtime.co/api/v4".to_string()),
            },
        })
    }
}
