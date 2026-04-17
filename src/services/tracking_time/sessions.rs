use anyhow::{Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use super::cache;

// ─── Paths ────────────────────────────────────────────────────────────────────

fn sessions_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("ttime-bot").join("sessions.json")
}

// ─── Structs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionStore {
    pub date: NaiveDate,
    pub current: Option<Session>,
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub meetings: Vec<Meeting>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Session {
    pub id: String,
    pub task_id: u64,
    pub task_name: String,
    pub project_id: Option<u64>,
    pub project_name: Option<String>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub duration_min: Option<u64>,
    pub notes: Option<String>,
    pub logged: bool,
    pub source: String,
}

/// Reunión importada desde Google Calendar.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub calendar_event_id: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_min: u64,
    pub attendees: Option<String>,
    /// Tarea TT asignada al confirmar la reunión
    pub task_id: Option<u64>,
    pub task_name: Option<String>,
    pub project_id: Option<u64>,
    pub project_name: Option<String>,
    /// El usuario confirmó que la reunión se realizó
    pub confirmed: bool,
    /// Duración real reportada por el usuario (puede diferir del calendario)
    pub actual_duration_min: Option<u64>,
    pub logged: bool,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Gap {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub duration_min: u64,
}

#[derive(Debug, Serialize)]
pub struct EodReview {
    pub date: String,
    pub open_session: Option<Session>,
    pub sessions: Vec<Session>,
    pub meetings_pending: Vec<Meeting>,    // sin confirmar
    pub meetings_confirmed: Vec<Meeting>,  // confirmadas (algunas pueden no estar logueadas)
    pub total_tracked_min: u64,
    pub gaps: Vec<Gap>,
    pub unlogged_count: usize,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

pub fn today_uy() -> NaiveDate {
    let uy = FixedOffset::west_opt(3 * 3600).expect("offset válido");
    Utc::now().with_timezone(&uy).date_naive()
}

/// Parsea "HH:MM" (hora de hoy en UTC-3) o un datetime completo.
pub(crate) fn parse_time_arg(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Ok(dt);
    }
    if let Some((h, m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse()?;
        let min: u32 = m.trim().parse()?;
        let uy = FixedOffset::west_opt(3 * 3600)
            .ok_or_else(|| anyhow::anyhow!("offset inválido"))?;
        let today_uy = Utc::now().with_timezone(&uy).date_naive();
        let naive = today_uy
            .and_hms_opt(hour, min, 0)
            .ok_or_else(|| anyhow::anyhow!("hora inválida: {}", s))?;
        return Ok(uy
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| anyhow::anyhow!("ambiguous datetime"))?
            .to_utc());
    }
    bail!("Formato de tiempo no reconocido: '{}'. Usá 'HH:MM' o datetime ISO", s)
}

fn gen_session_id(task_id: u64) -> String {
    let ms = Utc::now().timestamp_millis();
    format!("s_{}_{}", ms, task_id)
}

fn gen_meeting_id() -> String {
    let ms = Utc::now().timestamp_millis();
    format!("m_{}", ms)
}

// ─── Store I/O ────────────────────────────────────────────────────────────────

pub fn load_store() -> Result<SessionStore> {
    let path = sessions_path();
    if !path.exists() {
        return Ok(SessionStore {
            date: today_uy(),
            current: None,
            sessions: vec![],
            meetings: vec![],
        });
    }
    let data = std::fs::read_to_string(&path)?;
    let store: SessionStore = serde_json::from_str(&data)?;
    Ok(store)
}

pub fn save_store(store: &SessionStore) -> Result<()> {
    let path = sessions_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

fn reset_if_new_day(store: &mut SessionStore) {
    let today = today_uy();
    if store.date != today {
        store.date = today;
        store.current = None;
        store.sessions.clear();
        store.meetings.clear();
    }
}

// ─── Session operations ───────────────────────────────────────────────────────

/// Inicia una nueva sesión. Si hay una abierta, la cierra primero.
/// Retorna (sesión iniciada, sesión cerrada automáticamente si existía).
pub fn start_session(
    task_id: u64,
    task_name: String,
    project_id: Option<u64>,
    project_name: Option<String>,
    notes: Option<String>,
) -> Result<(Session, Option<Session>)> {
    let mut store = load_store()?;
    reset_if_new_day(&mut store);

    // Cerrar sesión abierta si existe
    let auto_closed = if let Some(mut prev) = store.current.take() {
        let now = Utc::now();
        prev.end = Some(now);
        let dur = (now - prev.start).num_minutes().max(0) as u64;
        prev.duration_min = Some(dur);
        store.sessions.push(prev.clone());
        Some(prev)
    } else {
        None
    };

    let new_session = Session {
        id: gen_session_id(task_id),
        task_id,
        task_name: task_name.clone(),
        project_id,
        project_name: project_name.clone(),
        start: Utc::now(),
        end: None,
        duration_min: None,
        notes,
        logged: false,
        source: "claude".to_string(),
    };
    store.current = Some(new_session.clone());
    save_store(&store)?;

    // Registrar tarea como conocida para búsquedas futuras
    if let (Some(pid), Some(pname)) = (project_id, project_name) {
        let _ = cache::save_known_task(task_id, &task_name, pid, &pname);
    }

    Ok((new_session, auto_closed))
}

/// Cierra la sesión activa y la mueve a `sessions`.
pub fn end_session(notes_override: Option<String>) -> Result<Session> {
    let mut store = load_store()?;
    reset_if_new_day(&mut store);

    let mut session = store
        .current
        .take()
        .ok_or_else(|| anyhow::anyhow!("No hay ninguna sesión activa en este momento"))?;

    let now = Utc::now();
    session.end = Some(now);
    let dur = (now - session.start).num_minutes().max(0) as u64;
    session.duration_min = Some(dur);
    if notes_override.is_some() {
        session.notes = notes_override;
    }

    store.sessions.push(session.clone());
    save_store(&store)?;
    Ok(session)
}

/// Marca una sesión o reunión como logueada en TrackingTime.
pub fn mark_logged(id: &str) -> Result<()> {
    let mut store = load_store()?;
    if let Some(s) = store.sessions.iter_mut().find(|s| s.id == id) {
        s.logged = true;
    } else if let Some(m) = store.meetings.iter_mut().find(|m| m.id == id) {
        m.logged = true;
    }
    save_store(&store)?;
    Ok(())
}

// ─── Meeting operations ───────────────────────────────────────────────────────

/// Importa un evento de Google Calendar al store del día.
/// Claude llama esto después de obtener los eventos del calendario.
pub fn import_meeting(
    title: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    calendar_event_id: Option<String>,
    attendees: Option<String>,
) -> Result<Meeting> {
    let mut store = load_store()?;
    reset_if_new_day(&mut store);

    // Evitar duplicados por calendar_event_id
    if let Some(ref cal_id) = calendar_event_id {
        if store.meetings.iter().any(|m| m.calendar_event_id.as_deref() == Some(cal_id.as_str())) {
            return store.meetings
                .iter()
                .find(|m| m.calendar_event_id.as_deref() == Some(cal_id.as_str()))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("inconsistencia interna"));
        }
    }

    let duration_min = (end - start).num_minutes().max(0) as u64;
    let meeting = Meeting {
        id: gen_meeting_id(),
        title,
        calendar_event_id,
        start,
        end,
        duration_min,
        attendees,
        task_id: None,
        task_name: None,
        project_id: None,
        project_name: None,
        confirmed: false,
        actual_duration_min: None,
        logged: false,
        source: "google_calendar".to_string(),
    };

    store.meetings.push(meeting.clone());
    save_store(&store)?;
    Ok(meeting)
}

/// Confirma que una reunión se realizó y la mapea a una tarea de TrackingTime.
pub fn confirm_meeting(
    meeting_id: &str,
    task_id: u64,
    task_name: String,
    project_id: Option<u64>,
    project_name: Option<String>,
    actual_duration_min: Option<u64>,
) -> Result<Meeting> {
    let mut store = load_store()?;

    let meeting = store.meetings.iter_mut()
        .find(|m| m.id == meeting_id)
        .ok_or_else(|| anyhow::anyhow!("Reunión '{}' no encontrada", meeting_id))?;

    meeting.confirmed = true;
    meeting.task_id = Some(task_id);
    meeting.task_name = Some(task_name);
    meeting.project_id = project_id;
    meeting.project_name = project_name;
    if let Some(d) = actual_duration_min {
        meeting.actual_duration_min = Some(d);
    }

    let result = meeting.clone();
    save_store(&store)?;

    // Registrar tarea como conocida para búsquedas futuras
    if let (Some(pid), Some(ref pname)) = (result.project_id, &result.project_name) {
        if let Some(ref tname) = result.task_name {
            let _ = cache::save_known_task(task_id, tname, pid, pname);
        }
    }

    Ok(result)
}

/// Carga el store y arma el resumen de fin de día.
pub fn eod_review(gap_start: Option<DateTime<Utc>>) -> Result<EodReview> {
    let mut store = load_store()?;
    reset_if_new_day(&mut store);

    // Sesión abierta — calculamos duración hasta ahora para mostrar
    let open_session = store.current.as_ref().map(|s| {
        let mut s2 = s.clone();
        let now = Utc::now();
        s2.end = Some(now);
        s2.duration_min = Some((now - s2.start).num_minutes().max(0) as u64);
        s2
    });

    // Tiempo total: sesiones completas + abierta + reuniones confirmadas
    let sessions_min: u64 = store.sessions.iter().filter_map(|s| s.duration_min).sum();
    let open_min = open_session.as_ref().and_then(|s| s.duration_min).unwrap_or(0);
    let meetings_min: u64 = store.meetings.iter()
        .filter(|m| m.confirmed)
        .map(|m| m.actual_duration_min.unwrap_or(m.duration_min))
        .sum();
    let total_tracked_min = sessions_min + open_min + meetings_min;

    // Gaps: cubrir con sesiones + open + reuniones confirmadas
    let gaps = compute_gaps_with_meetings(
        &store.sessions,
        open_session.as_ref(),
        &store.meetings,
        gap_start,
    );

    // Pendientes de loguear: sesiones no logueadas + reuniones confirmadas no logueadas + abierta
    let unlogged_count = store.sessions.iter().filter(|s| !s.logged).count()
        + store.meetings.iter().filter(|m| m.confirmed && !m.logged).count()
        + open_session.as_ref().map(|_| 1).unwrap_or(0);

    let (meetings_pending, meetings_confirmed): (Vec<_>, Vec<_>) =
        store.meetings.iter().cloned().partition(|m| !m.confirmed);

    Ok(EodReview {
        date: store.date.to_string(),
        open_session,
        sessions: store.sessions,
        meetings_pending,
        meetings_confirmed,
        total_tracked_min,
        gaps,
        unlogged_count,
    })
}

// ─── Gap computation ──────────────────────────────────────────────────────────

const MIN_GAP_MINUTES: u64 = 5;

/// Calcula gaps considerando sesiones Claude + reuniones confirmadas del calendario.
pub fn compute_gaps_with_meetings(
    sessions: &[Session],
    open: Option<&Session>,
    meetings: &[Meeting],
    gap_start: Option<DateTime<Utc>>,
) -> Vec<Gap> {
    let mut intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();

    // Sesiones completadas
    for s in sessions {
        if let Some(e) = s.end {
            intervals.push((s.start, e));
        }
    }
    // Sesión abierta
    if let Some(os) = open {
        if let Some(e) = os.end {
            intervals.push((os.start, e));
        }
    }
    // Reuniones confirmadas (usan duración real si fue ajustada)
    for m in meetings.iter().filter(|m| m.confirmed) {
        let dur = m.actual_duration_min.unwrap_or(m.duration_min);
        let end = m.start + chrono::Duration::minutes(dur as i64);
        intervals.push((m.start, end));
    }

    compute_gaps_from_intervals(intervals, gap_start)
}

fn compute_gaps_from_intervals(
    mut intervals: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    gap_start: Option<DateTime<Utc>>,
) -> Vec<Gap> {
    if intervals.is_empty() {
        return vec![];
    }

    intervals.sort_by_key(|(s, _)| *s);

    // Fusionar intervalos solapados
    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (s, e) in intervals {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    let range_start = gap_start.unwrap_or(merged[0].0);
    let range_end = merged.last().map(|(_, e)| *e).unwrap_or(Utc::now());

    let mut gaps = vec![];
    let mut cursor = range_start;

    for (seg_start, seg_end) in &merged {
        if *seg_start > cursor {
            let gap_min = (*seg_start - cursor).num_minutes().max(0) as u64;
            if gap_min >= MIN_GAP_MINUTES {
                gaps.push(Gap { from: cursor, to: *seg_start, duration_min: gap_min });
            }
        }
        if *seg_end > cursor {
            cursor = *seg_end;
        }
    }

    // Gap al final del día (solo si se indicó gap_start)
    if gap_start.is_some() && range_end < Utc::now() {
        let gap_min = (Utc::now() - range_end).num_minutes().max(0) as u64;
        if gap_min >= MIN_GAP_MINUTES {
            gaps.push(Gap { from: range_end, to: Utc::now(), duration_min: gap_min });
        }
    }

    gaps
}
