use anyhow::Result;
use reqwest::{Client, RequestBuilder, header};
use chrono::Utc;

use crate::config::{TrackingTimeAuth, TrackingTimeConfig};
use super::cache;
use super::models::*;
use super::sessions::parse_time_arg;
use super::cache::{save_known_task};

pub struct TrackingTimeClient {
    http: Client,
    base_url: String,
    auth: TrackingTimeAuth,
}

impl TrackingTimeClient {
    pub fn new(config: &TrackingTimeConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/json"));

        if let TrackingTimeAuth::Token(token) = &config.auth {
            let auth_value = header::HeaderValue::from_str(&format!("Token {}", token))?;
            headers.insert(header::AUTHORIZATION, auth_value);
        }

        let http = Client::builder().default_headers(headers).build()?;

        Ok(Self {
            http,
            base_url: config.base_url.clone(),
            auth: config.auth.clone(),
        })
    }

    /// Aplica autenticación Basic si corresponde (token ya va en el header por defecto).
    fn auth(&self, req: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            TrackingTimeAuth::Basic { email, password } => req.basic_auth(email, Some(password)),
            TrackingTimeAuth::Token(_) => req,
        }
    }

    // ─── Proyectos ────────────────────────────────────────────────────────────

    pub async fn list_projects(&self, force_refresh: bool) -> Result<Vec<Project>> {
        if !force_refresh {
            if let Some(cached) = cache::load_projects() {
                return Ok(cached);
            }
        }
        let response: ApiResponse<Vec<Project>> = self
            .auth(self.http.get(format!("{}/projects", self.base_url)))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let projects = response.data;
        let _ = cache::save_projects(&projects);
        Ok(projects)
    }

    // ─── Tareas ───────────────────────────────────────────────────────────────

    pub async fn list_tasks(&self, project_id: Option<u64>, force_refresh: bool) -> Result<Vec<Task>> {
        if !force_refresh {
            if let Some(cached) = cache::load_tasks(project_id) {
                return Ok(cached);
            }
        }

        let tasks = if let Some(pid) = project_id {
            // Endpoint correcto para tareas de un proyecto específico
            let response: ApiResponse<ProjectMin> = self
                .auth(self.http.get(format!("{}/projects/{}/min", self.base_url, pid)))
                .query(&[("include_tasks", "true")])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let project_name = response.data.name.clone();
            let mut tasks = response.data.tasks.unwrap_or_default();

            // Rellenar project_name en cada tarea (viene null desde este endpoint)
            for t in &mut tasks {
                t.project_name = Some(project_name.clone());
            }

            // Guardar en known_tasks para uso futuro como tareas recurrentes
            for t in &tasks {
                let _ = save_known_task(t.id, &t.name, pid, &project_name);
            }

            tasks
        } else {
            let mut req = self.auth(self.http.get(format!("{}/tasks", self.base_url)));
            req = req.query(&[("filter", "ALL")]);
            let response: ApiResponse<Vec<Task>> = req
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            response.data
        };

        let _ = cache::save_tasks(&tasks, project_id);
        Ok(tasks)
    }

    pub async fn create_task(&self, req: CreateTaskRequest) -> Result<Task> {
        let response: ApiResponse<Task> = self
            .auth(self.http.post(format!("{}/tasks", self.base_url)))
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response.data)
    }

    // ─── Entradas de tiempo ────────────────────────────────────────────────────

    pub async fn list_time_entries(&self, task_id: Option<u64>, limit: Option<u32>) -> Result<Vec<TimeEntry>> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(tid) = task_id {
            params.push(("tid".to_string(), tid.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit".to_string(), l.to_string()));
        }

        let raw: serde_json::Value = self
            .auth(self.http.get(format!("{}/events", self.base_url)))
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        // Los events usan campos abreviados: s=start, e=end, d=duration, tid=task_id, etc.
        let events = raw["data"].as_array().cloned().unwrap_or_default();
        let entries: Vec<TimeEntry> = events.into_iter().map(|ev| TimeEntry {
            id: ev["id"].as_u64().unwrap_or(0),
            task_id: ev["tid"].as_u64(),
            task_name: ev["t"].as_str().map(String::from),
            project_id: ev["pid"].as_u64(),
            project_name: ev["p"].as_str().map(String::from),
            start: Some(ev["s"].clone()),
            end: if ev["e"].is_null() { None } else { Some(ev["e"].clone()) },
            duration: ev["d"].as_u64(),
            notes: ev["n"].as_str().map(String::from),
        }).collect();

        Ok(entries)
    }

    pub async fn start_timer(&self, task_id: u64, notes: Option<String>) -> Result<TimeEntry> {
        let body = StartTimerRequest { task_id, notes };
        let response: ApiResponse<TimeEntry> = self
            .auth(self.http.post(format!("{}/events/add", self.base_url)))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response.data)
    }

    /// Registra tiempo pasado con start y end específicos (sin timer real-time).
    /// `start` y `end` son strings en formato "HH:MM" o "YYYY-MM-DDTHH:MM:SS".
    /// Si solo se provee hora (HH:MM), se asume la fecha de hoy en UTC-3 (Uruguay).
    pub async fn log_time(
        &self,
        task_id: u64,
        start_str: &str,
        end_str: &str,
        notes: Option<String>,
    ) -> Result<TimeEntry> {
        let start = parse_time_arg(start_str)?;
        let end = parse_time_arg(end_str)?;
        let duration_secs = (end - start).num_seconds().max(0) as u64;
        let body = LogTimeRequest { task_id, start, end, duration: duration_secs, notes };
        let response: ApiResponse<TimeEntry> = self
            .auth(self.http.post(format!("{}/events/add", self.base_url)))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response.data)
    }

    pub async fn stop_timer(&self, entry_id: u64) -> Result<TimeEntry> {
        let body = StopTimerRequest { end: Utc::now() };
        let response: ApiResponse<TimeEntry> = self
            .auth(self.http.put(format!("{}/events/{}", self.base_url, entry_id)))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response.data)
    }

    pub async fn get_active_timer(&self) -> Result<Option<TimeEntry>> {
        let entries: Vec<TimeEntry> = self.list_time_entries(None, Some(1)).await?;
        // Una entrada activa no tiene `end`
        Ok(entries.into_iter().find(|e| e.end.is_none()))
    }

    // ─── Resolución de tareas ─────────────────────────────────────────────────

    /// Busca una tarea por ID en cache local; si no está, refresca y reintenta.
    pub async fn resolve_task(&self, task_id: u64) -> Result<Option<Task>> {
        if let Some(tasks) = cache::load_tasks(None) {
            if let Some(t) = tasks.into_iter().find(|t| t.id == task_id) {
                return Ok(Some(t));
            }
        }
        let tasks = self.list_tasks(None, true).await?;
        Ok(tasks.into_iter().find(|t| t.id == task_id))
    }

    // ─── Usuarios ─────────────────────────────────────────────────────────────

    pub async fn list_users(&self) -> Result<Vec<User>> {
        let response: ApiResponse<Vec<User>> = self
            .auth(self.http.get(format!("{}/users", self.base_url)))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response.data)
    }
}
