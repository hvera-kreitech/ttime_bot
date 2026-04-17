use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Respuesta genérica de la API ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub response: ApiMeta,
}

#[derive(Debug, Deserialize)]
pub struct ApiMeta {
    pub status: u16,
    pub err: Option<String>,
}

// ─── Proyectos ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub color: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectMin {
    pub id: Option<u64>,
    pub name: String,
    pub tasks: Option<Vec<Task>>,
}

// ─── Tareas ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Task {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    pub project_id: Option<u64>,
    pub project_name: Option<String>,
    // el API puede devolver status como número o string
    pub status: Option<serde_json::Value>,
    // puede venir como número entero (minutos) o float
    pub estimated_hours: Option<serde_json::Value>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub project_id: Option<u64>,
    pub notes: Option<String>,
    pub estimated_hours: Option<f64>,
}

// ─── Entradas de tiempo ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TimeEntry {
    #[serde(default)]
    pub id: u64,
    pub task_id: Option<u64>,
    #[serde(alias = "task")]
    pub task_name: Option<String>,
    pub project_id: Option<u64>,
    #[serde(alias = "project")]
    pub project_name: Option<String>,
    pub start: Option<serde_json::Value>,
    pub end: Option<serde_json::Value>,
    pub duration: Option<u64>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartTimerRequest {
    pub task_id: u64,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StopTimerRequest {
    pub end: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct LogTimeRequest {
    pub task_id: u64,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration: u64,
    pub notes: Option<String>,
}

// ─── Usuarios ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub role: Option<String>,
}
