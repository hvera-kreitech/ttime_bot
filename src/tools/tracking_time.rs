use rmcp::{
    model::{CallToolResult, Content, Tool},
    Error as McpError,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::instrument;

use crate::services::tracking_time::{
    client::TrackingTimeClient,
    fuzzy,
    models::CreateTaskRequest,
    sessions,
};

/// Conjunto de MCP tools que exponen las capacidades de TrackingTime
pub struct TrackingTimeTools {
    client: Arc<TrackingTimeClient>,
    /// Token del usuario en modo HTTP; None en modo stdio
    user_token: Option<String>,
}

/// Helper: construye un JSON Schema "object" para el input_schema de una Tool
fn schema(
    properties: Option<serde_json::Value>,
    required: Option<&[&'static str]>,
) -> Arc<serde_json::Map<String, Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), json!("object"));
    if let Some(props) = properties {
        map.insert("properties".into(), props);
    }
    if let Some(req) = required {
        map.insert("required".into(), json!(req));
    }
    Arc::new(map)
}

impl TrackingTimeTools {
    pub fn new(client: Arc<TrackingTimeClient>, user_token: Option<String>) -> Self {
        Self { client, user_token }
    }

    /// Retorna la lista de tools disponibles para registrar en el servidor MCP
    pub fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool::new(
                "tt_setup",
                "Configura las credenciales de TrackingTime para este usuario. \
                 Valida el email y app_password contra la API, obtiene el account_id \
                 automáticamente y guarda la configuración de forma persistente. \
                 Usar esta tool la primera vez o cuando cambien las credenciales. \
                 El app_password se genera en: https://app.trackingtime.co/settings/api",
                schema(
                    Some(json!({
                        "email": {
                            "type": "string",
                            "description": "Email de la cuenta de TrackingTime"
                        },
                        "app_password": {
                            "type": "string",
                            "description": "App Password generado en TrackingTime (Settings → API)"
                        }
                    })),
                    Some(&["email", "app_password"]),
                ),
            ),
            Tool::new(
                "tt_list_projects",
                "Lista todos los proyectos disponibles en TrackingTime. \
                 Sirve el resultado desde cache local; usar force_refresh=true \
                 solo cuando un proyecto esperado no aparece en la lista (puede ser nuevo).",
                schema(
                    Some(json!({
                        "force_refresh": {
                            "type": "boolean",
                            "description": "Si es true, ignora el cache y consulta la API. \
                                            Usar solo cuando un proyecto no se encuentra en la lista."
                        }
                    })),
                    None,
                ),
            ),
            Tool::new(
                "tt_list_tasks",
                "Lista las tareas de TrackingTime. Se puede filtrar por proyecto. \
                 Sirve desde cache local; usar force_refresh=true si una tarea esperada no aparece.",
                schema(
                    Some(json!({
                        "project_id": {
                            "type": "number",
                            "description": "ID del proyecto para filtrar tareas (opcional)"
                        },
                        "force_refresh": {
                            "type": "boolean",
                            "description": "Si es true, ignora el cache y consulta la API. \
                                            Usar solo cuando una tarea no se encuentra en la lista."
                        }
                    })),
                    None,
                ),
            ),
            Tool::new(
                "tt_create_task",
                "Crea una nueva tarea en TrackingTime.",
                schema(
                    Some(json!({
                        "name": {
                            "type": "string",
                            "description": "Nombre de la tarea"
                        },
                        "project_id": {
                            "type": "number",
                            "description": "ID del proyecto al que pertenece la tarea (opcional)"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Notas adicionales sobre la tarea (opcional)"
                        },
                        "estimated_hours": {
                            "type": "number",
                            "description": "Horas estimadas para completar la tarea (opcional)"
                        }
                    })),
                    Some(&["name"]),
                ),
            ),
            Tool::new(
                "tt_start_timer",
                "Inicia el timer para una tarea específica. \
                 Si hay un timer activo, primero debe detenerse con tt_stop_timer.",
                schema(
                    Some(json!({
                        "task_id": {
                            "type": "number",
                            "description": "ID de la tarea a cronometrar"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Notas sobre lo que se va a trabajar (opcional)"
                        }
                    })),
                    Some(&["task_id"]),
                ),
            ),
            Tool::new(
                "tt_stop_timer",
                "Detiene el timer activo. Retorna la entrada de tiempo registrada.",
                schema(
                    Some(json!({
                        "entry_id": {
                            "type": "number",
                            "description": "ID de la entrada de tiempo a detener"
                        }
                    })),
                    Some(&["entry_id"]),
                ),
            ),
            Tool::new(
                "tt_get_active_timer",
                "Obtiene el timer que está corriendo actualmente, si existe.",
                schema(None, None),
            ),
            Tool::new(
                "tt_list_time_entries",
                "Lista las entradas de tiempo registradas. \
                 Permite ver el historial de trabajo por tarea.",
                schema(
                    Some(json!({
                        "task_id": {
                            "type": "number",
                            "description": "Filtrar por ID de tarea (opcional)"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Cantidad máxima de entradas a retornar (opcional, default 20)"
                        }
                    })),
                    None,
                ),
            ),
            Tool::new(
                "tt_log_time",
                "Registra tiempo trabajado en forma retroactiva (con hora de inicio y fin \
                 específicas). Ideal para loguear horas ya realizadas sin haber usado el timer \
                 en tiempo real. Acepta 'HH:MM' (hora de hoy en Uruguay UTC-3) o un datetime \
                 ISO completo.",
                schema(
                    Some(json!({
                        "task_id": {
                            "type": "number",
                            "description": "ID de la tarea"
                        },
                        "start": {
                            "type": "string",
                            "description": "Hora de inicio: 'HH:MM' o 'YYYY-MM-DDTHH:MM:SSZ'"
                        },
                        "end": {
                            "type": "string",
                            "description": "Hora de fin: 'HH:MM' o 'YYYY-MM-DDTHH:MM:SSZ'"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Notas sobre el trabajo realizado (opcional)"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "ID de sesión local a marcar como logueada tras registrar (opcional)"
                        }
                    })),
                    Some(&["task_id", "start", "end"]),
                ),
            ),
            Tool::new(
                "tt_start_session",
                "Inicia una sesión de trabajo local para trackear tiempo mientras se trabaja \
                 con Claude. Si hay una sesión abierta, la cierra automáticamente primero. \
                 No inicia el timer en TrackingTime; acumula la sesión para el resumen de fin de día.",
                schema(
                    Some(json!({
                        "task_id": {
                            "type": "number",
                            "description": "ID de la tarea en la que se va a trabajar"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Descripción breve de lo que se va a hacer (opcional)"
                        }
                    })),
                    Some(&["task_id"]),
                ),
            ),
            Tool::new(
                "tt_end_session",
                "Cierra la sesión de trabajo activa y calcula la duración automáticamente. \
                 La sesión queda pendiente de loguear hasta la revisión de fin de día.",
                schema(
                    Some(json!({
                        "notes": {
                            "type": "string",
                            "description": "Notas sobre lo que se hizo en la sesión (opcional)"
                        }
                    })),
                    None,
                ),
            ),
            Tool::new(
                "tt_eod_review",
                "Revisión de fin de día: muestra todas las sesiones del día, tiempo total \
                 trackeado, gaps (períodos sin actividad) y sesiones pendientes de subir a \
                 TrackingTime. Usar para confirmar y loguear el trabajo del día.",
                schema(
                    Some(json!({
                        "gap_start": {
                            "type": "string",
                            "description": "Hora desde la que detectar gaps, ej: '09:00'. \
                                            Si se omite, se empieza desde la primera sesión del día."
                        }
                    })),
                    None,
                ),
            ),
            Tool::new(
                "tt_mark_logged",
                "Marca una sesión o reunión local como ya logueada en TrackingTime. \
                 Llamar después de un tt_log_time exitoso para mantener el estado sincronizado.",
                schema(
                    Some(json!({
                        "session_id": {
                            "type": "string",
                            "description": "ID de la sesión o reunión local a marcar como logueada"
                        }
                    })),
                    Some(&["session_id"]),
                ),
            ),
            Tool::new(
                "tt_find_task",
                "Busca proyectos y tareas en cache local por nombre en lenguaje natural. \
                 Ideal para resolver a qué tarea de TrackingTime corresponde una descripción \
                 libre del usuario (ej: 'drot desarrollo', 'reunión MI', 'bps sprint'). \
                 Devuelve candidatos con score de relevancia para que el usuario confirme.",
                schema(
                    Some(json!({
                        "query": {
                            "type": "string",
                            "description": "Texto de búsqueda libre, ej: 'DROT desarrollo' o 'reunión ministerio interior'"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Máximo de resultados a devolver (default 5)"
                        }
                    })),
                    Some(&["query"]),
                ),
            ),
            Tool::new(
                "tt_resolve_work",
                "Resuelve el proyecto y la tarea a partir de descripciones en lenguaje natural. \
                 Implementa el flujo completo: busca el proyecto en cache (fuzzy), si no está \
                 consulta la API; luego busca la tarea dentro del proyecto en cache, si no está \
                 consulta la API. Devuelve project_id y task_id listos para usar en tt_start_timer \
                 o tt_log_time. Usar esta tool como punto de entrada cuando el usuario dice en qué \
                 proyecto y tarea está trabajando.",
                schema(
                    Some(json!({
                        "project": {
                            "type": "string",
                            "description": "Nombre del proyecto (puede tener errores ortográficos o ser parcial), ej: 'drot traslados', 'BPS', 'ministerio interior'"
                        },
                        "task": {
                            "type": "string",
                            "description": "Descripción de la tarea (lenguaje natural), ej: 'desarrollo', 'reunión con el cliente', 'planning'"
                        }
                    })),
                    Some(&["project", "task"]),
                ),
            ),
            Tool::new(
                "tt_import_meeting",
                "Importa un evento de Google Calendar al registro del día. \
                 Claude llama esta tool por cada reunión obtenida del calendario. \
                 Las reuniones quedan pendientes hasta que el usuario las confirme con tt_confirm_meeting.",
                schema(
                    Some(json!({
                        "title": {
                            "type": "string",
                            "description": "Título del evento del calendario"
                        },
                        "start": {
                            "type": "string",
                            "description": "Hora de inicio: 'HH:MM' o datetime ISO"
                        },
                        "end": {
                            "type": "string",
                            "description": "Hora de fin: 'HH:MM' o datetime ISO"
                        },
                        "calendar_event_id": {
                            "type": "string",
                            "description": "ID del evento en Google Calendar (para evitar duplicados)"
                        },
                        "attendees": {
                            "type": "string",
                            "description": "Lista de participantes separados por coma (opcional)"
                        }
                    })),
                    Some(&["title", "start", "end"]),
                ),
            ),
            Tool::new(
                "tt_confirm_meeting",
                "Confirma que una reunión del calendario se realizó y la asocia a una tarea \
                 de TrackingTime. Opcionalmente ajusta la duración real si fue distinta a la del calendario.",
                schema(
                    Some(json!({
                        "meeting_id": {
                            "type": "string",
                            "description": "ID de la reunión (obtenido de tt_import_meeting o tt_eod_review)"
                        },
                        "task_id": {
                            "type": "number",
                            "description": "ID de la tarea en TrackingTime donde loguear la reunión"
                        },
                        "actual_duration_min": {
                            "type": "number",
                            "description": "Duración real en minutos (si fue distinta a la del calendario)"
                        }
                    })),
                    Some(&["meeting_id", "task_id"]),
                ),
            ),
        ]
    }

    /// Dispatcher principal: recibe el nombre de la tool y sus argumentos
    #[instrument(skip(self), fields(tool = %name))]
    pub async fn call(&self, name: &str, args: Value) -> Result<CallToolResult, McpError> {
        match name {
            "tt_setup" => self.setup(args).await,
            "tt_list_projects" => self.list_projects(args).await,
            "tt_list_tasks" => self.list_tasks(args).await,
            "tt_create_task" => self.create_task(args).await,
            "tt_start_timer" => self.start_timer(args).await,
            "tt_stop_timer" => self.stop_timer(args).await,
            "tt_get_active_timer" => self.get_active_timer().await,
            "tt_list_time_entries" => self.list_time_entries(args).await,
            "tt_log_time" => self.log_time(args).await,
            "tt_start_session" => self.start_session(args).await,
            "tt_end_session" => self.end_session(args).await,
            "tt_eod_review" => self.eod_review(args).await,
            "tt_mark_logged" => self.mark_logged(args).await,
            "tt_find_task" => self.find_task(args).await,
            "tt_resolve_work" => self.resolve_work(args).await,
            "tt_import_meeting" => self.import_meeting(args).await,
            "tt_confirm_meeting" => self.confirm_meeting(args).await,
            _ => Err(McpError::invalid_params(
                format!("Tool desconocida: {}", name),
                None,
            )),
        }
    }

    // ─── Implementaciones individuales ────────────────────────────────────────

    async fn setup(&self, args: Value) -> Result<CallToolResult, McpError> {
        let email = args.get("email")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'email' es requerido", None))?
            .to_string();
        let app_password = args.get("app_password")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'app_password' es requerido", None))?
            .to_string();

        let (account_id, name) = TrackingTimeClient::validate_credentials(&email, &app_password)
            .await
            .map_err(|e| McpError::internal_error(
                format!("Credenciales inválidas: {}", e), None
            ))?;

        let base_url = format!("https://api.trackingtime.co/api/v4/{}", account_id);
        let user_cfg = super::super::services::tracking_time::cache::UserConfig {
            email: email.clone(),
            password: app_password,
            base_url: base_url.clone(),
        };

        // En modo HTTP guardar por token; en stdio guardar config global
        if let Some(token) = &self.user_token {
            super::super::services::tracking_time::cache::save_user_config_by_token(token, &user_cfg)
        } else {
            super::super::services::tracking_time::cache::save_user_config(&user_cfg)
        }.map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Configuración guardada correctamente.\n\
             Usuario: {} ({})\n\
             Account ID: {}\n\
             Base URL: {}\n\n\
             A partir de ahora usaré estas credenciales automáticamente.",
            name, email, account_id, base_url
        ))]))
    }

    async fn list_projects(&self, args: Value) -> Result<CallToolResult, McpError> {
        let force_refresh = args.get("force_refresh").and_then(|v| v.as_bool()).unwrap_or(false);

        let projects = self.client.list_projects(force_refresh).await.map_err(|e| {
            McpError::internal_error(e.to_string(), None)
        })?;

        let text = serde_json::to_string_pretty(&projects)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn list_tasks(&self, args: Value) -> Result<CallToolResult, McpError> {
        let project_id = args.get("project_id").and_then(|v| v.as_u64());
        let force_refresh = args.get("force_refresh").and_then(|v| v.as_bool()).unwrap_or(false);

        let tasks = self.client.list_tasks(project_id, force_refresh).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&tasks)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn create_task(&self, args: Value) -> Result<CallToolResult, McpError> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'name' es requerido", None))?
            .to_string();

        let request = CreateTaskRequest {
            name,
            project_id: args.get("project_id").and_then(|v| v.as_u64()),
            notes: args.get("notes").and_then(|v| v.as_str()).map(String::from),
            estimated_hours: args.get("estimated_hours").and_then(|v| v.as_f64()),
        };

        let task = self.client.create_task(request).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&task)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn start_timer(&self, args: Value) -> Result<CallToolResult, McpError> {
        let task_id = args.get("task_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| McpError::invalid_params("El campo 'task_id' es requerido", None))?;

        let notes = args.get("notes").and_then(|v| v.as_str()).map(String::from);

        let entry = self.client.start_timer(task_id, notes).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&entry)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Timer iniciado.\n\n{}",
            text
        ))]))
    }

    async fn stop_timer(&self, args: Value) -> Result<CallToolResult, McpError> {
        let entry_id = args.get("entry_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| McpError::invalid_params("El campo 'entry_id' es requerido", None))?;

        let entry = self.client.stop_timer(entry_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&entry)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Timer detenido.\n\n{}",
            text
        ))]))
    }

    async fn get_active_timer(&self) -> Result<CallToolResult, McpError> {
        let active = self.client.get_active_timer().await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = match active {
            Some(entry) => serde_json::to_string_pretty(&entry)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            None => "No hay ningún timer activo en este momento.".to_string(),
        };

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn log_time(&self, args: Value) -> Result<CallToolResult, McpError> {
        let task_id = args.get("task_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| McpError::invalid_params("El campo 'task_id' es requerido", None))?;

        let start = args.get("start")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'start' es requerido", None))?;

        let end = args.get("end")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'end' es requerido", None))?;

        let notes = args.get("notes").and_then(|v| v.as_str()).map(String::from);
        let session_id = args.get("session_id").and_then(|v| v.as_str()).map(String::from);

        let entry = self.client.log_time(task_id, start, end, notes).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        // Marcar sesión local como logueada si se proporcionó session_id
        if let Some(sid) = session_id {
            let _ = sessions::mark_logged(&sid);
        }

        let duration_min = entry.duration.map(|d| d / 60);
        let text = serde_json::to_string_pretty(&entry)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let summary = if let Some(min) = duration_min {
            format!("Tiempo registrado: {}h {}min.\n\n{}", min / 60, min % 60, text)
        } else {
            format!("Tiempo registrado.\n\n{}", text)
        };

        Ok(CallToolResult::success(vec![Content::text(summary)]))
    }

    async fn list_time_entries(&self, args: Value) -> Result<CallToolResult, McpError> {
        let task_id = args.get("task_id").and_then(|v| v.as_u64());
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);

        let entries = self.client
            .list_time_entries(task_id, limit.or(Some(20)))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // ─── Sesiones locales ─────────────────────────────────────────────────────

    async fn start_session(&self, args: Value) -> Result<CallToolResult, McpError> {
        let task_id = args.get("task_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| McpError::invalid_params("El campo 'task_id' es requerido", None))?;

        let notes = args.get("notes").and_then(|v| v.as_str()).map(String::from);

        // Resolver nombre de tarea y proyecto desde cache/API
        let task = self.client.resolve_task(task_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (task_name, project_id, project_name) = match task {
            Some(t) => (t.name, t.project_id, t.project_name),
            None => (format!("Tarea #{}", task_id), None, None),
        };

        let (session, auto_closed) = sessions::start_session(
            task_id, task_name.clone(), project_id, project_name.clone(), notes,
        ).map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let uy = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        let start_local = session.start.with_timezone(&uy).format("%H:%M").to_string();

        let mut msg = format!(
            "Sesión iniciada: **{}**{} a las {} (UY).\nID: `{}`",
            task_name,
            project_name.as_deref().map(|p| format!(" ({})", p)).unwrap_or_default(),
            start_local,
            session.id,
        );

        if let Some(prev) = auto_closed {
            let dur = prev.duration_min.unwrap_or(0);
            msg.push_str(&format!(
                "\n\n⚠️ Se cerró automáticamente la sesión anterior: **{}** ({} min).",
                prev.task_name, dur
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    async fn end_session(&self, args: Value) -> Result<CallToolResult, McpError> {
        let notes = args.get("notes").and_then(|v| v.as_str()).map(String::from);

        let session = sessions::end_session(notes)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let uy = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        let start_local = session.start.with_timezone(&uy).format("%H:%M").to_string();
        let end_local = session.end
            .map(|e| e.with_timezone(&uy).format("%H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());
        let dur = session.duration_min.unwrap_or(0);

        let msg = format!(
            "Sesión cerrada: **{}** — {} → {} ({} min).\nID: `{}`\nPendiente de loguear en TrackingTime.",
            session.task_name, start_local, end_local, dur, session.id
        );

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    async fn eod_review(&self, args: Value) -> Result<CallToolResult, McpError> {
        let gap_start = args.get("gap_start")
            .and_then(|v| v.as_str())
            .and_then(|s| sessions::parse_time_arg(s).ok());

        let review = sessions::eod_review(gap_start)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&review)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn mark_logged(&self, args: Value) -> Result<CallToolResult, McpError> {
        let session_id = args.get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'session_id' es requerido", None))?;

        sessions::mark_logged(session_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(
            format!("`{}` marcado como logueado.", session_id)
        )]))
    }

    async fn find_task(&self, args: Value) -> Result<CallToolResult, McpError> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'query' es requerido", None))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        // Si cache de tareas está vacío, refrescarlo desde la API primero
        if super::super::services::tracking_time::cache::load_tasks(None).is_none() {
            let _ = self.client.list_tasks(None, false).await;
        }

        let results = fuzzy::search(query, limit);

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                format!("No se encontraron coincidencias para '{}'. Intentá con tt_list_projects para ver todos los proyectos.", query)
            )]));
        }

        let text = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Resultados para '{}':\n\n{}",
            query, text
        ))]))
    }

    async fn resolve_work(&self, args: Value) -> Result<CallToolResult, McpError> {
        let project_query = args.get("project")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'project' es requerido", None))?
            .to_string();
        let task_query = args.get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'task' es requerido", None))?
            .to_string();

        // ── Paso 1: resolver proyecto ────────────────────────────────────────────

        // Intentar desde cache primero
        let project_match = fuzzy::find_project(&project_query);

        let project_match = if project_match.is_none() {
            // Cache vacío o sin match → refrescar desde API
            self.client.list_projects(true).await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            fuzzy::find_project(&project_query)
        } else {
            project_match
        };

        let project = match project_match {
            Some(p) => p,
            None => {
                // Listar proyectos disponibles para orientar al usuario
                let projects = super::super::services::tracking_time::cache::load_projects()
                    .unwrap_or_default();
                let names: Vec<String> = projects.iter()
                    .map(|p| format!("• {} (id: {})", p.name, p.id))
                    .collect();
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "No encontré ningún proyecto que coincida con '{}'. \
                     ¿Quisiste decir alguno de estos?\n\n{}",
                    project_query,
                    names.join("\n")
                ))]));
            }
        };

        // ── Paso 2: resolver tarea dentro del proyecto ───────────────────────────

        // Intentar desde cache de ese proyecto
        let task_match = fuzzy::find_task_in_project(&task_query, project.project_id, &project.project_name);

        let task_match = if task_match.is_none() {
            // No hay cache para este proyecto → fetchear tareas
            self.client.list_tasks(Some(project.project_id), false).await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            fuzzy::find_task_in_project(&task_query, project.project_id, &project.project_name)
        } else {
            task_match
        };

        match task_match {
            Some(task) => {
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "✓ Proyecto: {} (id: {})\n✓ Tarea: {} (id: {})\n\n\
                     Podés usar task_id={} en tt_start_timer o tt_log_time.",
                    project.project_name, project.project_id,
                    task.task_name, task.task_id,
                    task.task_id
                ))]))
            }
            None => {
                // Listar tareas disponibles del proyecto para orientar al usuario
                let tasks = super::super::services::tracking_time::cache::load_tasks(Some(project.project_id))
                    .unwrap_or_default();
                let names: Vec<String> = tasks.iter()
                    .map(|t| format!("• {} (id: {})", t.name, t.id))
                    .collect();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Proyecto encontrado: {} (id: {})\n\
                     Pero no encontré tarea que coincida con '{}'.\n\n\
                     Tareas disponibles en este proyecto:\n{}",
                    project.project_name, project.project_id,
                    task_query,
                    if names.is_empty() { "  (sin tareas en cache)".to_string() } else { names.join("\n") }
                ))]))
            }
        }
    }

    async fn import_meeting(&self, args: Value) -> Result<CallToolResult, McpError> {
        let title = args.get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'title' es requerido", None))?
            .to_string();

        let start_str = args.get("start")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'start' es requerido", None))?;
        let end_str = args.get("end")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'end' es requerido", None))?;

        let start = sessions::parse_time_arg(start_str)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let end = sessions::parse_time_arg(end_str)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let calendar_event_id = args.get("calendar_event_id").and_then(|v| v.as_str()).map(String::from);
        let attendees = args.get("attendees").and_then(|v| v.as_str()).map(String::from);

        let meeting = sessions::import_meeting(title, start, end, calendar_event_id, attendees)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let uy = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        let start_local = meeting.start.with_timezone(&uy).format("%H:%M").to_string();
        let end_local = meeting.end.with_timezone(&uy).format("%H:%M").to_string();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Reunión importada: **{}** — {} → {} ({} min)\nID: `{}`\nPendiente de confirmar con tt_confirm_meeting.",
            meeting.title, start_local, end_local, meeting.duration_min, meeting.id
        ))]))
    }

    async fn confirm_meeting(&self, args: Value) -> Result<CallToolResult, McpError> {
        let meeting_id = args.get("meeting_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("El campo 'meeting_id' es requerido", None))?;

        let task_id = args.get("task_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| McpError::invalid_params("El campo 'task_id' es requerido", None))?;

        let actual_duration_min = args.get("actual_duration_min").and_then(|v| v.as_u64());

        // Resolver nombre de tarea/proyecto desde cache
        let task = self.client.resolve_task(task_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (task_name, project_id, project_name) = match task {
            Some(t) => (t.name, t.project_id, t.project_name),
            None => (format!("Tarea #{}", task_id), None, None),
        };

        let meeting = sessions::confirm_meeting(
            meeting_id, task_id, task_name, project_id, project_name, actual_duration_min,
        ).map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let dur = meeting.actual_duration_min.unwrap_or(meeting.duration_min);
        let uy = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        let start_local = meeting.start.with_timezone(&uy).format("%H:%M").to_string();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Reunión confirmada: **{}** → {} ({} min) desde las {}.\nPendiente de loguear con tt_log_time.",
            meeting.title,
            meeting.task_name.as_deref().unwrap_or("?"),
            dur,
            start_local,
        ))]))
    }
}
