use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::models::{Project, Task};

// ─── Configuración de usuario ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserConfig {
    pub email: String,
    pub password: String,
    pub base_url: String,
}

pub fn load_user_config() -> Option<UserConfig> {
    let data = std::fs::read_to_string(cache_dir().join("config.json")).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_user_config(config: &UserConfig) -> Result<()> {
    let path = cache_dir().join("config.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

// ─── Tareas conocidas (índice persistente) ────────────────────────────────────

/// Tarea con nombre de proyecto, para búsqueda fuzzy offline.
/// Se agregan aquí automáticamente cuando el usuario trabaja en ellas.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnownTask {
    pub id: u64,
    pub name: String,
    pub project_id: u64,
    pub project_name: String,
}

pub fn load_known_tasks() -> Vec<KnownTask> {
    let data = std::fs::read_to_string(cache_dir().join("known_tasks.json")).ok();
    data.and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default()
}

pub fn save_known_task(task_id: u64, task_name: &str, project_id: u64, project_name: &str) -> Result<()> {
    let mut tasks = load_known_tasks();
    // Actualizar si ya existe, agregar si no
    if let Some(existing) = tasks.iter_mut().find(|t| t.id == task_id) {
        existing.name = task_name.to_string();
        existing.project_name = project_name.to_string();
    } else {
        tasks.push(KnownTask {
            id: task_id,
            name: task_name.to_string(),
            project_id,
            project_name: project_name.to_string(),
        });
    }
    let path = cache_dir().join("known_tasks.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(&tasks)?)?;
    Ok(())
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("ttime-bot")
}

// ─── Proyectos ────────────────────────────────────────────────────────────────

pub fn load_projects() -> Option<Vec<Project>> {
    let data = std::fs::read_to_string(cache_dir().join("projects.json")).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_projects(projects: &[Project]) -> Result<()> {
    let path = cache_dir().join("projects.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(projects)?)?;
    Ok(())
}

// ─── Tareas ───────────────────────────────────────────────────────────────────

pub fn load_tasks(project_id: Option<u64>) -> Option<Vec<Task>> {
    let filename = match project_id {
        Some(id) => format!("tasks_{}.json", id),
        None => "tasks.json".to_string(),
    };
    let data = std::fs::read_to_string(cache_dir().join(&filename)).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_tasks(tasks: &[Task], project_id: Option<u64>) -> Result<()> {
    let filename = match project_id {
        Some(id) => format!("tasks_{}.json", id),
        None => "tasks.json".to_string(),
    };
    let path = cache_dir().join(&filename);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(tasks)?)?;
    Ok(())
}
