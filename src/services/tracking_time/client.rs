use anyhow::Result;
use reqwest::{Client, RequestBuilder, header};
use chrono::{DateTime, FixedOffset, Utc};

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

    // ─── Setup ───────────────────────────────────────────────────────────────

    /// Valida credenciales contra la API y devuelve el account_id del usuario.
    /// No requiere una instancia inicializada — se usa durante tt_setup.
    pub async fn validate_credentials(email: &str, password: &str) -> Result<(u64, String)> {
        let client = reqwest::Client::new();
        let response: serde_json::Value = client
            .get("https://api.trackingtime.co/api/v4/me")
            .basic_auth(email, Some(password))
            .header("Content-Type", "application/json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let account_id = response["data"]["account_id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("No se pudo obtener account_id de la respuesta"))?;
        let name = response["data"]["name"]
            .as_str()
            .unwrap_or("Usuario")
            .to_string();

        Ok((account_id, name))
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
            let raw = self
                .auth(self.http.get(format!("{}/projects/{}/min", self.base_url, pid)))
                .query(&[("include_tasks", "true")])
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            tracing::debug!("ProjectMin raw response (first 500): {}", &raw[..raw.len().min(500)]);
            let response: ApiResponse<ProjectMin> = serde_json::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("Error deserializando ProjectMin: {}\nBody (1000 chars): {}", e, &raw[..raw.len().min(1000)]))?;

            let project_name = response.data.name.clone().unwrap_or_default();
            let mut tasks = response.data.tasks.unwrap_or_default();

            // Rellenar project_name en cada tarea (viene null desde este endpoint)
            for t in &mut tasks {
                t.project_name = Some(project_name.clone());
            }

            // Guardar en known_tasks para uso futuro como tareas recurrentes
            for t in &tasks {
                let _ = save_known_task(t.id, t.name.as_deref().unwrap_or(""), pid, &project_name);
            }

            tasks
        } else {
            let mut req = self.auth(self.http.get(format!("{}/tasks", self.base_url)));
            req = req.query(&[("filter", "ALL")]);
            let raw = req.send().await?.error_for_status()?.text().await?;
            tracing::debug!("Tasks raw response (first 500): {}", &raw[..raw.len().min(500)]);
            let response: ApiResponse<Vec<Task>> = serde_json::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("Error deserializando Vec<Task>: {}\nBody (1000 chars): {}", e, &raw[..raw.len().min(1000)]))?;
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

    pub async fn list_time_entries(
        &self,
        task_id: Option<u64>,
        project_id: Option<u64>,
        since: Option<&str>,
        until: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<TimeEntry>> {
        // /events/flat es el endpoint correcto para filtrar por proyecto o tarea con rango de fechas.
        // /events (sin account_id en el path) solo sirve para obtener el timer activo.
        if project_id.is_some() || task_id.is_some() || since.is_some() {
            return self.list_time_entries_flat(task_id, project_id, since, until, limit).await;
        }

        // Sin filtros: usar /events para obtener entradas recientes (ej: timer activo)
        let mut params: Vec<(String, String)> = Vec::new();
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

        let events = raw["data"].as_array().cloned().unwrap_or_default();
        Ok(events.into_iter().map(parse_event).collect())
    }

    /// Llama a /events/flat con filter=PROJECT o filter=TASK y rango from/to.
    async fn list_time_entries_flat(
        &self,
        task_id: Option<u64>,
        project_id: Option<u64>,
        since: Option<&str>,
        until: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<TimeEntry>> {
        let mut params: Vec<(String, String)> = Vec::new();

        if let Some(pid) = project_id {
            params.push(("filter".to_string(), "PROJECT".to_string()));
            params.push(("id".to_string(), pid.to_string()));
        } else if let Some(tid) = task_id {
            params.push(("filter".to_string(), "TASK".to_string()));
            params.push(("id".to_string(), tid.to_string()));
        }

        if let Some(s) = since {
            params.push(("from".to_string(), s.to_string()));
        }
        if let Some(u) = until {
            params.push(("to".to_string(), u.to_string()));
        }
        if let Some(l) = limit {
            params.push(("page_size".to_string(), l.to_string()));
        }
        params.push(("include_custom_fields".to_string(), "true".to_string()));

        let raw: serde_json::Value = self
            .auth(self.http.get(format!("{}/events/flat", self.base_url)))
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        tracing::debug!("events/flat response (first 500): {}", &raw.to_string()[..raw.to_string().len().min(500)]);

        let events = raw["data"].as_array().cloned().unwrap_or_default();
        Ok(events.into_iter().map(parse_flat_event).collect())
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
        let start_tt = to_tt_datetime(start);
        let end_tt = to_tt_datetime(end);
        let date_tt = start_tt[..10].to_string(); // "YYYY-MM-DD"
        let body = LogTimeRequest {
            task_id,
            date: date_tt,
            start: start_tt,
            end: end_tt,
            duration: duration_secs,
            notes,
        };

        let body_json = serde_json::to_string_pretty(&body)?;
        std::fs::write("/tmp/ttime-bot-log-time-req.log", &body_json).ok();
        tracing::debug!("log_time request body: {}", &body_json);

        let resp = self
            .auth(self.http.post(format!("{}/events/add", self.base_url)))
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let resp_body = resp.text().await?;
        std::fs::write(
            "/tmp/ttime-bot-log-time-res.log",
            format!("status: {status}\nbody:\n{resp_body}"),
        ).ok();
        tracing::debug!("log_time response: status={}, body={}", status, &resp_body);

        if !status.is_success() {
            return Err(anyhow::anyhow!("log_time falló: status {}, body: {}", status, resp_body));
        }

        let response: ApiResponse<TimeEntry> = serde_json::from_str(&resp_body)
            .map_err(|e| anyhow::anyhow!("Error deserializando log_time response: {}\nBody: {}", e, resp_body))?;
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
        let entries: Vec<TimeEntry> = self.list_time_entries(None, None, None, None, Some(1)).await?;
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

/// Parsea un evento del endpoint /events (campos abreviados: s, e, d, tid, pid, t, p, n).
fn parse_event(ev: serde_json::Value) -> TimeEntry {
    TimeEntry {
        id: ev["id"].as_u64().unwrap_or(0),
        task_id: ev["tid"].as_u64(),
        task_name: ev["t"].as_str().map(String::from),
        project_id: ev["pid"].as_u64(),
        project_name: ev["p"].as_str().map(String::from),
        start: Some(ev["s"].clone()),
        end: if ev["e"].is_null() { None } else { Some(ev["e"].clone()) },
        duration: ev["d"].as_u64(),
        notes: ev["n"].as_str().map(String::from),
    }
}

/// Parsea un evento del endpoint /events/flat.
/// Los campos usan nombres capitalizados con espacios: "ID", "Start", "Task Id", etc.
fn parse_flat_event(ev: serde_json::Value) -> TimeEntry {
    TimeEntry {
        id: ev["ID"].as_u64().unwrap_or(0),
        task_id: ev["Task Id"].as_u64(),
        task_name: ev["Task"].as_str().map(String::from),
        project_id: ev["Project Id"].as_u64(),
        project_name: ev["Project"].as_str().map(String::from),
        start: ev.get("Start").filter(|v| !v.is_null()).cloned(),
        end: ev.get("End").filter(|v| !v.is_null()).cloned(),
        duration: ev["Duration"].as_u64(),
        notes: ev["Notes"].as_str().map(String::from),
    }
}

/// Convierte un `DateTime<Utc>` al formato que espera la API de TrackingTime:
/// "YYYY-MM-DD HH:MM:SS" en hora local Uruguay (UTC-3, sin DST).
fn to_tt_datetime(dt: DateTime<Utc>) -> String {
    let uy = FixedOffset::west_opt(3 * 3600).expect("offset válido");
    dt.with_timezone(&uy).format("%Y-%m-%d %H:%M:%S").to_string()
}
