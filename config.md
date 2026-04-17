# ttime-bot — Configuración y Arquitectura

## Qué es esto

Servidor **MCP (Model Context Protocol)** escrito en Rust que conecta modelos de IA
(Claude y a futuro otros) con servicios externos. El primer servicio integrado es
**TrackingTime** para gestión inteligente de horas.

## Arquitectura

```
Claude Desktop / Claude Code
        │
        │  MCP Protocol (stdio)
        ▼
┌───────────────────┐
│   ttime-bot       │  ← Este proyecto (Rust)
│                   │
│  MCP Server       │
│  ├─ tt_* tools    │──────► TrackingTime API
│  └─ (futuro)      │──────► Otros servicios
└───────────────────┘
```

### Estructura de directorios

```
src/
├── main.rs                    # Servidor MCP, entry point
├── config.rs                  # Variables de entorno
├── error.rs                   # Tipos de error
├── services/
│   ├── mod.rs
│   └── tracking_time/
│       ├── mod.rs
│       ├── client.rs          # Cliente HTTP de la API
│       └── models.rs          # Structs de request/response
└── tools/
    ├── mod.rs
    └── tracking_time.rs       # Definición e implementación de MCP tools
```

## Setup inicial

### 1. Prerequisitos

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Variables de entorno

```bash
cp .env.example .env
# Editar .env y poner el token de TrackingTime
```

El token de TrackingTime se obtiene en:
`https://app.trackingtime.co/settings/api`

### 3. Build

```bash
cargo build --release
```

El binario queda en `target/release/ttime-bot`.

### 4. Conectar con Claude Desktop

Editar `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "ttime-bot": {
      "command": "/ruta/absoluta/al/target/release/ttime-bot",
      "env": {
        "TRACKING_TIME_API_TOKEN": "tu_token_aqui"
      }
    }
  }
}
```

### 5. Conectar con Claude Code (CLI)

```bash
claude mcp add ttime-bot /ruta/absoluta/al/target/release/ttime-bot \
  --env TRACKING_TIME_API_TOKEN=tu_token_aqui
```

## Tools disponibles

| Tool | Descripción |
|------|-------------|
| `tt_list_projects` | Lista todos los proyectos |
| `tt_list_tasks` | Lista tareas (filtrable por proyecto) |
| `tt_create_task` | Crea una nueva tarea |
| `tt_start_timer` | Inicia el timer en una tarea |
| `tt_stop_timer` | Detiene el timer activo |
| `tt_get_active_timer` | Consulta si hay un timer corriendo |
| `tt_list_time_entries` | Historial de entradas de tiempo |
| `tt_log_time` | Registra tiempo retroactivo con hora de inicio y fin (ej: "13:30", "14:15") |

## Ejemplos de uso con Claude

Una vez conectado, puedes pedirle a Claude:

- _"¿Qué proyectos tengo en TrackingTime?"_
- _"Crea una tarea llamada 'Revisar PRs' en el proyecto Backend"_
- _"Inicia el timer para la tarea 42"_
- _"¿Cuántas horas llevo hoy?"_ (requiere implementar filtros de fecha)
- _"Para el timer y dime cuánto tiempo trabajé"_

## Agregar nuevos servicios

1. Crear `src/services/nuevo_servicio/` con `client.rs` y `models.rs`
2. Crear `src/tools/nuevo_servicio.rs` con las tool definitions
3. En `main.rs`, agregar el prefijo al dispatcher en `call_tool`
4. En `list_tools`, extender con las nuevas tool definitions

El sistema de prefijos (`tt_`, `gh_`, etc.) permite escalar sin conflictos de nombres.

## Roadmap

- [ ] Resúmenes de horas por período (diario / semanal / mensual)
- [ ] Filtros de tiempo en `tt_list_time_entries` (desde/hasta)
- [ ] Tool para reportes de productividad
- [ ] Integración con otros servicios (GitHub, Jira, etc.)
- [ ] Transport HTTP/SSE además de stdio (para acceso remoto)
