use anyhow::{Context, Result};
use crate::services::tracking_time::cache;

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
    /// Carga config para modo HTTP, identificando al usuario por token.
    pub fn from_token(token: &str) -> Result<Self> {
        let user_cfg = cache::load_user_config_by_token(token)
            .ok_or_else(|| anyhow::anyhow!(
                "No hay credenciales para este token. Ejecuta tt_setup para configurar tu cuenta."
            ))?;
        Ok(Config {
            tracking_time: TrackingTimeConfig {
                auth: TrackingTimeAuth::Basic {
                    email: user_cfg.email,
                    password: user_cfg.password,
                },
                base_url: user_cfg.base_url,
            },
        })
    }

    /// Config vacía para modo HTTP antes de que el usuario ejecute tt_setup.
    pub fn unconfigured() -> Self {
        Config {
            tracking_time: TrackingTimeConfig {
                auth: TrackingTimeAuth::Basic {
                    email: String::new(),
                    password: String::new(),
                },
                base_url: "https://api.trackingtime.co/api/v4".to_string(),
            },
        }
    }

    pub fn from_env() -> Result<Self> {
        // Prioridad: archivo de config > env vars
        if let Some(user_cfg) = cache::load_user_config() {
            return Ok(Config {
                tracking_time: TrackingTimeConfig {
                    auth: TrackingTimeAuth::Basic {
                        email: user_cfg.email,
                        password: user_cfg.password,
                    },
                    base_url: user_cfg.base_url,
                },
            });
        }

        // Fallback a env vars
        let auth = if let Ok(token) = std::env::var("TRACKING_TIME_API_TOKEN") {
            TrackingTimeAuth::Token(token)
        } else {
            TrackingTimeAuth::Basic {
                email: std::env::var("TRACKING_TIME_EMAIL")
                    .context("Credenciales no configuradas. Usa tt_setup para configurar tu cuenta.")?,
                password: std::env::var("TRACKING_TIME_PASSWORD")
                    .context("TRACKING_TIME_PASSWORD requerido cuando no hay API token")?,
            }
        };
        Ok(Config {
            tracking_time: TrackingTimeConfig {
                auth,
                base_url: std::env::var("TRACKING_TIME_BASE_URL")
                    .unwrap_or_else(|_| "https://api.trackingtime.co/api/v4".to_string()),
            },
        })
    }
}
