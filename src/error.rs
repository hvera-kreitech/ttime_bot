use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Error en la API de TrackingTime: {0}")]
    TrackingTimeApi(String),

    #[error("Error de configuración: {0}")]
    Config(String),

    #[error("Error de red: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Error de serialización: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type AppResult<T> = std::result::Result<T, AppError>;
