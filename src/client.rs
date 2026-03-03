use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::model::{HostInfo, SessionDirection, SessionEventType, SessionEventV1};

#[derive(Debug, Parser)]
#[command(name = "rustdesk-cli")]
#[command(about = "CLI MVP para control de grabacion, eventos y presencia")]
pub struct Cli {
    #[arg(long, global = true)]
    pub server_url: Option<String>,
    #[arg(long, global = true)]
    pub user_id: Option<String>,
    #[arg(long, global = true)]
    pub direction: Option<SessionDirectionArg>,
    #[arg(long, global = true)]
    pub recording_mode: Option<RecordingMode>,
    #[arg(long, global = true)]
    pub recording_incoming: Option<OnOff>,
    #[arg(long, global = true)]
    pub recording_outgoing: Option<OnOff>,
    #[arg(long, global = true)]
    pub recording_storage_path: Option<PathBuf>,
    #[arg(long, global = true)]
    pub config_path: Option<PathBuf>,
    #[arg(long, global = true)]
    pub state_path: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    fn as_bool(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingMode {
    Off,
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionDirectionArg {
    Incoming,
    Outgoing,
}

impl From<SessionDirectionArg> for SessionDirection {
    fn from(value: SessionDirectionArg) -> Self {
        match value {
            SessionDirectionArg::Incoming => SessionDirection::Incoming,
            SessionDirectionArg::Outgoing => SessionDirection::Outgoing,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Recording {
        #[command(subcommand)]
        command: RecordingCommand,
    },
    Presence {
        #[command(subcommand)]
        command: PresenceCommand,
    },
    Show,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    Start {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        direction: Option<SessionDirectionArg>,
    },
    End {
        #[arg(long)]
        session_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RecordingCommand {
    Start {
        #[arg(long)]
        session_id: String,
    },
    Stop {
        #[arg(long)]
        session_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum PresenceCommand {
    Join {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        participant_id: Option<String>,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        avatar_url: Option<String>,
        #[arg(long)]
        direction: Option<SessionDirectionArg>,
    },
    Leave {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        participant_id: Option<String>,
        #[arg(long)]
        direction: Option<SessionDirectionArg>,
    },
    Control {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        participant_id: String,
        #[arg(long)]
        direction: Option<SessionDirectionArg>,
    },
    Activity {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        participant_id: Option<String>,
        #[arg(long)]
        signal: Option<String>,
        #[arg(long)]
        direction: Option<SessionDirectionArg>,
    },
    Show {
        #[arg(long)]
        session_id: String,
    },
    Sessions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_url: String,
    pub user_id: String,
    pub default_direction: SessionDirectionArg,
    pub recording_mode: RecordingMode,
    pub recording_incoming: bool,
    pub recording_outgoing: bool,
    pub recording_storage_path: Option<PathBuf>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        let user_id = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "operator".to_string());

        Self {
            server_url: "http://127.0.0.1:8080".to_string(),
            user_id,
            default_direction: SessionDirectionArg::Outgoing,
            recording_mode: RecordingMode::Manual,
            recording_incoming: true,
            recording_outgoing: true,
            recording_storage_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeState {
    pub active_sessions: HashMap<String, ActiveSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSession {
    pub direction: SessionDirectionArg,
    pub recording_active: bool,
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let (config_path, state_path) = resolve_paths(cli.config_path.clone(), cli.state_path.clone())?;

    let mut config = load_or_default::<ClientConfig>(&config_path)?;
    apply_overrides(&mut config, &cli);
    save_json_file(&config_path, &config)?;

    let mut state = load_or_default::<RuntimeState>(&state_path)?;

    match cli.command {
        Some(Command::Session { command }) => {
            handle_session_command(command, &config, &mut state).await?;
            save_json_file(&state_path, &state)?;
        }
        Some(Command::Recording { command }) => {
            handle_recording_command(command, &config, &mut state).await?;
            save_json_file(&state_path, &state)?;
        }
        Some(Command::Presence { command }) => {
            handle_presence_command(command, &config, &state).await?;
        }
        Some(Command::Show) | None => {
            let snapshot = json!({
                "config": config,
                "state": state,
                "paths": {
                    "config_path": config_path,
                    "state_path": state_path,
                }
            });
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
    }

    Ok(())
}

fn resolve_paths(
    explicit_config_path: Option<PathBuf>,
    explicit_state_path: Option<PathBuf>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let base_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustdesk-cli");

    let config_path = explicit_config_path.unwrap_or_else(|| base_dir.join("config.json"));
    let state_path = explicit_state_path.unwrap_or_else(|| base_dir.join("state.json"));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create state directory: {}", parent.display()))?;
    }

    Ok((config_path, state_path))
}

fn apply_overrides(config: &mut ClientConfig, cli: &Cli) {
    if let Some(server_url) = &cli.server_url {
        config.server_url = server_url.clone();
    }
    if let Some(user_id) = &cli.user_id {
        config.user_id = user_id.clone();
    }
    if let Some(direction) = cli.direction {
        config.default_direction = direction;
    }
    if let Some(recording_mode) = cli.recording_mode {
        config.recording_mode = recording_mode;
    }
    if let Some(toggle) = cli.recording_incoming {
        config.recording_incoming = toggle.as_bool();
    }
    if let Some(toggle) = cli.recording_outgoing {
        config.recording_outgoing = toggle.as_bool();
    }
    if let Some(path) = &cli.recording_storage_path {
        config.recording_storage_path = Some(path.clone());
    }
}

async fn handle_session_command(
    command: SessionCommand,
    config: &ClientConfig,
    state: &mut RuntimeState,
) -> anyhow::Result<()> {
    match command {
        SessionCommand::Start {
            session_id,
            direction,
        } => {
            if state.active_sessions.contains_key(&session_id) {
                anyhow::bail!("session {session_id} is already active");
            }

            let direction = direction.unwrap_or(config.default_direction);
            let resolved_direction: SessionDirection = direction.into();

            send_event(config, SessionEventType::SessionStarted, &session_id, resolved_direction, None)
                .await?;

            let self_presence_meta = presence_identity_meta(&config.user_id, Some(config.user_id.clone()), None);
            send_event(
                config,
                SessionEventType::ParticipantJoined,
                &session_id,
                resolved_direction,
                Some(self_presence_meta.clone()),
            )
            .await?;
            send_event(
                config,
                SessionEventType::ControlChanged,
                &session_id,
                resolved_direction,
                Some(self_presence_meta),
            )
            .await?;

            let mut recording_active = false;
            if config.recording_mode == RecordingMode::Auto
                && direction_allows_recording(config, direction)
            {
                let meta = recording_meta(config);
                send_event(
                    config,
                    SessionEventType::RecordingStarted,
                    &session_id,
                    resolved_direction,
                    Some(meta),
                )
                .await?;
                recording_active = true;
            }

            state.active_sessions.insert(
                session_id.clone(),
                ActiveSession {
                    direction,
                    recording_active,
                },
            );

            info!(session_id, ?direction, recording_active, "session started");
        }
        SessionCommand::End { session_id } => {
            let active = state
                .active_sessions
                .get(&session_id)
                .cloned()
                .with_context(|| format!("session {session_id} is not active"))?;
            let resolved_direction: SessionDirection = active.direction.into();

            if active.recording_active {
                let meta = recording_meta(config);
                send_event(
                    config,
                    SessionEventType::RecordingStopped,
                    &session_id,
                    resolved_direction,
                    Some(meta),
                )
                .await?;
            }

            let self_presence_meta = presence_identity_meta(&config.user_id, Some(config.user_id.clone()), None);
            send_event(
                config,
                SessionEventType::ParticipantLeft,
                &session_id,
                resolved_direction,
                Some(self_presence_meta),
            )
            .await?;

            send_event(config, SessionEventType::SessionEnded, &session_id, resolved_direction, None)
                .await?;

            state.active_sessions.remove(&session_id);
            info!(session_id, "session ended");
        }
    }

    Ok(())
}

async fn handle_recording_command(
    command: RecordingCommand,
    config: &ClientConfig,
    state: &mut RuntimeState,
) -> anyhow::Result<()> {
    match command {
        RecordingCommand::Start { session_id } => {
            if config.recording_mode == RecordingMode::Off {
                anyhow::bail!("recording_mode=off blocks manual recording start");
            }

            let session = state
                .active_sessions
                .get_mut(&session_id)
                .with_context(|| format!("session {session_id} is not active"))?;

            if session.recording_active {
                anyhow::bail!("recording already active for session {session_id}");
            }

            if !direction_allows_recording(config, session.direction) {
                anyhow::bail!("recording disabled for {:?} sessions", session.direction);
            }

            let resolved_direction: SessionDirection = session.direction.into();
            let meta = recording_meta(config);
            send_event(
                config,
                SessionEventType::RecordingStarted,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            session.recording_active = true;
            info!(session_id, "recording started");
        }
        RecordingCommand::Stop { session_id } => {
            let session = state
                .active_sessions
                .get_mut(&session_id)
                .with_context(|| format!("session {session_id} is not active"))?;

            if !session.recording_active {
                anyhow::bail!("recording is not active for session {session_id}");
            }

            let resolved_direction: SessionDirection = session.direction.into();
            let meta = recording_meta(config);
            send_event(
                config,
                SessionEventType::RecordingStopped,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            session.recording_active = false;
            info!(session_id, "recording stopped");
        }
    }

    Ok(())
}

async fn handle_presence_command(
    command: PresenceCommand,
    config: &ClientConfig,
    state: &RuntimeState,
) -> anyhow::Result<()> {
    match command {
        PresenceCommand::Join {
            session_id,
            participant_id,
            display_name,
            avatar_url,
            direction,
        } => {
            let participant_id = participant_id.unwrap_or_else(|| config.user_id.clone());
            let resolved_direction =
                resolve_presence_direction(state, &session_id, direction, config.default_direction);
            let meta = presence_identity_meta(&participant_id, display_name, avatar_url);

            send_event(
                config,
                SessionEventType::ParticipantJoined,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            info!(session_id, participant_id, "participant joined");
        }
        PresenceCommand::Leave {
            session_id,
            participant_id,
            direction,
        } => {
            let participant_id = participant_id.unwrap_or_else(|| config.user_id.clone());
            let resolved_direction =
                resolve_presence_direction(state, &session_id, direction, config.default_direction);
            let meta = presence_identity_meta(&participant_id, None, None);

            send_event(
                config,
                SessionEventType::ParticipantLeft,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            info!(session_id, participant_id, "participant left");
        }
        PresenceCommand::Control {
            session_id,
            participant_id,
            direction,
        } => {
            let resolved_direction =
                resolve_presence_direction(state, &session_id, direction, config.default_direction);
            let meta = presence_identity_meta(&participant_id, None, None);

            send_event(
                config,
                SessionEventType::ControlChanged,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            info!(session_id, participant_id, "control changed");
        }
        PresenceCommand::Activity {
            session_id,
            participant_id,
            signal,
            direction,
        } => {
            let participant_id = participant_id.unwrap_or_else(|| config.user_id.clone());
            let resolved_direction =
                resolve_presence_direction(state, &session_id, direction, config.default_direction);

            let mut meta = presence_identity_meta(&participant_id, None, None);
            if let Some(signal) = signal {
                meta["activity_signal"] = Value::String(signal);
            }

            send_event(
                config,
                SessionEventType::ParticipantActivity,
                &session_id,
                resolved_direction,
                Some(meta),
            )
            .await?;

            info!(session_id, participant_id, "participant activity sent");
        }
        PresenceCommand::Show { session_id } => {
            let endpoint = format!(
                "{}/api/v1/sessions/{}/presence",
                config.server_url.trim_end_matches('/'),
                session_id,
            );
            print_json_endpoint(&endpoint).await?;
        }
        PresenceCommand::Sessions => {
            let endpoint = format!(
                "{}/api/v1/sessions/presence",
                config.server_url.trim_end_matches('/')
            );
            print_json_endpoint(&endpoint).await?;
        }
    }

    Ok(())
}

fn resolve_presence_direction(
    state: &RuntimeState,
    session_id: &str,
    explicit_direction: Option<SessionDirectionArg>,
    default_direction: SessionDirectionArg,
) -> SessionDirection {
    let direction = explicit_direction
        .or_else(|| state.active_sessions.get(session_id).map(|s| s.direction))
        .unwrap_or(default_direction);
    direction.into()
}

fn direction_allows_recording(config: &ClientConfig, direction: SessionDirectionArg) -> bool {
    match direction {
        SessionDirectionArg::Incoming => config.recording_incoming,
        SessionDirectionArg::Outgoing => config.recording_outgoing,
    }
}

fn recording_meta(config: &ClientConfig) -> Value {
    json!({
        "recording_storage_path": config
            .recording_storage_path
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

fn presence_identity_meta(
    participant_id: &str,
    display_name: Option<String>,
    avatar_url: Option<String>,
) -> Value {
    json!({
        "participant_id": participant_id,
        "display_name": display_name,
        "avatar_url": avatar_url,
    })
}

async fn print_json_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .get(endpoint)
        .send()
        .await
        .with_context(|| format!("failed to query {endpoint}"))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .with_context(|| format!("failed to parse JSON from {endpoint}"))?;

    println!("{}", serde_json::to_string_pretty(&body)?);

    if !status.is_success() {
        anyhow::bail!("server returned non-success status {}", status);
    }

    Ok(())
}

async fn send_event(
    config: &ClientConfig,
    event_type: SessionEventType,
    session_id: &str,
    direction: SessionDirection,
    meta: Option<Value>,
) -> anyhow::Result<()> {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown-host".to_string());

    let event = SessionEventV1 {
        event_id: Uuid::new_v4(),
        event_type,
        session_id: session_id.to_string(),
        user_id: config.user_id.clone(),
        direction,
        timestamp: Utc::now(),
        host_info: Some(HostInfo {
            hostname: host,
            os: std::env::consts::OS.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }),
        meta,
    };

    let endpoint = format!(
        "{}/api/v1/session-events",
        config.server_url.trim_end_matches('/')
    );

    let response = reqwest::Client::new()
        .post(&endpoint)
        .json(&event)
        .send()
        .await
        .with_context(|| format!("failed to send event to {endpoint}"))?;

    if response.status() == reqwest::StatusCode::CONFLICT {
        anyhow::bail!("server rejected duplicated event_id: {}", event.event_id);
    }

    if response.status() != reqwest::StatusCode::ACCEPTED {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        warn!(status = %status, body, "unexpected server response");
        anyhow::bail!("server returned unexpected status {status}");
    }

    Ok(())
}

fn load_or_default<T>(path: &Path) -> anyhow::Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read file: {}", path.display()))?;
    let parsed = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in file: {}", path.display()))?;
    Ok(parsed)
}

fn save_json_file<T>(path: &Path, value: &T) -> anyhow::Result<()>
where
    T: Serialize,
{
    let serialized = serde_json::to_string_pretty(value).context("failed to serialize JSON file")?;
    std::fs::write(path, serialized)
        .with_context(|| format!("failed to write file: {}", path.display()))?;
    Ok(())
}
