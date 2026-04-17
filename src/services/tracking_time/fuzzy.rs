use serde::Serialize;
use super::cache;

// ─── Resultado de búsqueda ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct SearchResult {
    pub kind: String,           // "task" | "project"
    pub task_id: Option<u64>,
    pub task_name: Option<String>,
    pub project_id: u64,
    pub project_name: String,
    pub score: f32,
}

// ─── Búsqueda por proyecto ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProjectMatch {
    pub project_id: u64,
    pub project_name: String,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct TaskMatch {
    pub task_id: u64,
    pub task_name: String,
    pub project_id: u64,
    pub project_name: String,
    pub score: f32,
}

/// Busca el proyecto más parecido al query dentro del cache de proyectos.
/// Devuelve el mejor match si su score supera el umbral mínimo.
pub fn find_project(query: &str) -> Option<ProjectMatch> {
    let query_words = tokenize(query);
    if query_words.is_empty() { return None; }

    cache::load_projects()?.into_iter()
        .filter_map(|p| {
            let score = score_match(&query_words, &p.name);
            if score >= 0.25 {
                Some(ProjectMatch { project_id: p.id, project_name: p.name, score })
            } else {
                None
            }
        })
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
}

/// Busca la tarea más parecida al query dentro del cache de un proyecto específico.
/// Prioriza known_tasks, luego cache de API para ese proyecto.
pub fn find_task_in_project(query: &str, project_id: u64, project_name: &str) -> Option<TaskMatch> {
    let query_words = tokenize(query);
    if query_words.is_empty() { return None; }

    let mut best: Option<TaskMatch> = None;

    // Primero buscar en known_tasks (más confiables)
    for kt in cache::load_known_tasks() {
        if kt.project_id != project_id { continue; }
        let score = score_match(&query_words, &kt.name);
        if score >= 0.2 {
            let candidate = TaskMatch {
                task_id: kt.id,
                task_name: kt.name,
                project_id: kt.project_id,
                project_name: kt.project_name,
                score: score + 0.05,
            };
            if best.as_ref().map_or(true, |b| candidate.score > b.score) {
                best = Some(candidate);
            }
        }
    }

    // Luego en cache de API para ese proyecto
    if let Some(tasks) = cache::load_tasks(Some(project_id)) {
        for t in tasks {
            if t.id == 0 { continue; }
            if best.as_ref().map_or(false, |b| b.task_id == t.id) { continue; }
            let score = score_match(&query_words, &t.name);
            if score >= 0.2 {
                let candidate = TaskMatch {
                    task_id: t.id,
                    task_name: t.name,
                    project_id,
                    project_name: project_name.to_string(),
                    score,
                };
                if best.as_ref().map_or(true, |b| candidate.score > b.score) {
                    best = Some(candidate);
                }
            }
        }
    }

    best
}

// ─── Búsqueda principal ───────────────────────────────────────────────────────

/// Busca proyectos y tareas conocidas por nombre fuzzy.
/// Fuentes: known_tasks.json (tareas usadas antes) → tasks cache API → projects cache.
/// Devuelve hasta `limit` resultados ordenados por score descendente.
pub fn search(query: &str, limit: usize) -> Vec<SearchResult> {
    let query_words = tokenize(query);
    if query_words.is_empty() {
        return vec![];
    }

    let mut results: Vec<SearchResult> = Vec::new();

    // 1. Tareas conocidas (mayor prioridad — siempre disponibles)
    for kt in cache::load_known_tasks() {
        let combined = format!("{} {}", kt.name, kt.project_name);
        let score = score_match(&query_words, &combined);
        if score >= 0.2 {
            results.push(SearchResult {
                kind: "task".to_string(),
                task_id: Some(kt.id),
                task_name: Some(kt.name),
                project_id: kt.project_id,
                project_name: kt.project_name,
                score: score + 0.05, // bonus por ser conocida
            });
        }
    }

    // 2. Tareas del cache de la API (100 más recientes)
    if let Some(tasks) = cache::load_tasks(None) {
        for task in tasks {
            if task.id == 0 { continue; }
            // Evitar duplicado con known_tasks
            if results.iter().any(|r| r.task_id == Some(task.id)) { continue; }
            let project_name = task.project_name.clone().unwrap_or_default();
            let combined = format!("{} {}", task.name, project_name);
            let score = score_match(&query_words, &combined);
            if score >= 0.2 {
                results.push(SearchResult {
                    kind: "task".to_string(),
                    task_id: Some(task.id),
                    task_name: Some(task.name),
                    project_id: task.project_id.unwrap_or(0),
                    project_name,
                    score,
                });
            }
        }
    }

    // 3. Proyectos del cache (fallback cuando no hay tarea específica)
    if let Some(projects) = cache::load_projects() {
        for project in projects {
            let score = score_match(&query_words, &project.name);
            if score >= 0.25 {
                let has_good_task = results.iter().any(|r| {
                    r.project_id == project.id && r.score >= score
                });
                if !has_good_task {
                    results.push(SearchResult {
                        kind: "project".to_string(),
                        task_id: None,
                        task_name: None,
                        project_id: project.id,
                        project_name: project.name,
                        score,
                    });
                }
            }
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Eliminar proyectos redundantes si ya hay una tarea del mismo proyecto con score ≥
    let task_project_ids: std::collections::HashSet<u64> = results.iter()
        .filter(|r| r.task_id.is_some())
        .map(|r| r.project_id)
        .collect();
    results.retain(|r| r.task_id.is_some() || !task_project_ids.contains(&r.project_id));

    results.truncate(limit);
    results
}

// ─── Scoring ──────────────────────────────────────────────────────────────────

fn score_match(query_words: &[String], candidate: &str) -> f32 {
    let cand_words = tokenize(candidate);
    if cand_words.is_empty() {
        return 0.0;
    }

    // Match exacto del texto completo normalizado
    let norm_cand = cand_words.join(" ");
    let norm_query = query_words.join(" ");
    if norm_cand == norm_query {
        return 1.0;
    }

    // Contar palabras del query que aparecen en el candidato
    let matched: usize = query_words.iter().filter(|qw| {
        cand_words.iter().any(|cw| cw.contains(qw.as_str()) || qw.contains(cw.as_str()))
    }).count();

    let coverage = matched as f32 / query_words.len() as f32;

    // Bonus si el candidato empieza con el query o lo contiene entero
    let starts_bonus = if norm_cand.starts_with(&norm_query) { 0.15 } else { 0.0 };
    let contains_bonus = if norm_cand.contains(&norm_query) { 0.10 } else { 0.0 };

    (coverage * 0.75 + starts_bonus + contains_bonus).min(1.0)
}

// ─── Tokenización ─────────────────────────────────────────────────────────────

/// Convierte texto en tokens normalizados: minúsculas, sin acentos, sin stop words.
fn tokenize(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &["de", "la", "el", "los", "las", "del", "y", "en", "a", "por"];

    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| remove_diacritics(s.to_lowercase().as_str()))
        .filter(|s| s.len() >= 2 && !STOP_WORDS.contains(&s.as_str()))
        .collect()
}

/// Elimina tildes y diacríticos básicos del español.
fn remove_diacritics(s: &str) -> String {
    s.chars().map(|c| match c {
        'á' | 'à' | 'ä' => 'a',
        'é' | 'è' | 'ë' => 'e',
        'í' | 'ì' | 'ï' => 'i',
        'ó' | 'ò' | 'ö' => 'o',
        'ú' | 'ù' | 'ü' => 'u',
        'ñ' => 'n',
        'Á' | 'À' | 'Ä' => 'a',
        'É' | 'È' | 'Ë' => 'e',
        'Í' | 'Ì' | 'Ï' => 'i',
        'Ó' | 'Ò' | 'Ö' => 'o',
        'Ú' | 'Ù' | 'Ü' => 'u',
        'Ñ' => 'n',
        _ => c,
    }).collect()
}
