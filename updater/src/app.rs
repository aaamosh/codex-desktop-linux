use crate::{
    builder,
    cli::{Cli, Commands},
    config::{RuntimeConfig, RuntimePaths},
    logging,
    state::{PersistedState, UpdateStatus},
    upstream,
};
use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use tokio::time::{self, Duration};
use tracing::{error, info};

pub async fn run(cli: Cli) -> Result<()> {
    let paths = RuntimePaths::detect()?;
    paths.ensure_dirs()?;
    logging::init(&paths.log_file)?;

    let config = RuntimeConfig::load_or_default(&paths)?;
    let mut state =
        PersistedState::load_or_default(&paths.state_file, config.auto_install_on_app_exit)?;

    match cli.command {
        Commands::Daemon => run_daemon(&config, &mut state, &paths).await,
        Commands::CheckNow => run_check_now(&config, &mut state, &paths).await,
        Commands::Status { json } => run_status(state, json),
        Commands::InstallDeb { path } => run_install_deb(path, &mut state, &paths).await,
    }
}

async fn run_daemon(
    config: &RuntimeConfig,
    state: &mut PersistedState,
    paths: &RuntimePaths,
) -> Result<()> {
    state.auto_install_on_app_exit = config.auto_install_on_app_exit;
    state.save(&paths.state_file)?;
    info!("daemon initialized");

    time::sleep(Duration::from_secs(config.initial_check_delay_seconds)).await;
    run_check_cycle(config, state, paths).await?;

    let mut interval = time::interval(Duration::from_secs(config.check_interval_hours * 60 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = run_check_cycle(config, state, paths).await {
                    error!(?error, "periodic check failed");
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal?;
                info!("daemon received shutdown signal");
                break;
            }
        }
    }

    Ok(())
}

async fn run_check_now(
    config: &RuntimeConfig,
    state: &mut PersistedState,
    paths: &RuntimePaths,
) -> Result<()> {
    run_check_cycle(config, state, paths).await
}

fn run_status(state: PersistedState, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&state)?);
    } else {
        println!("status: {:?}", state.status);
        println!("installed_version: {}", state.installed_version);
        println!(
            "candidate_version: {}",
            state.candidate_version.as_deref().unwrap_or("none")
        );
    }

    Ok(())
}

async fn run_install_deb(
    path: std::path::PathBuf,
    state: &mut PersistedState,
    paths: &RuntimePaths,
) -> Result<()> {
    anyhow::ensure!(path.exists(), "Debian package not found: {}", path.display());
    state.artifact_paths.deb_path = Some(path);
    state.save(&paths.state_file)?;
    info!("install-deb scaffold initialized");
    Ok(())
}

async fn run_check_cycle(
    config: &RuntimeConfig,
    state: &mut PersistedState,
    paths: &RuntimePaths,
) -> Result<()> {
    let client = Client::builder().build()?;

    state.auto_install_on_app_exit = config.auto_install_on_app_exit;
    state.status = UpdateStatus::CheckingUpstream;
    state.last_check_at = Some(Utc::now());
    state.error_message = None;
    state.save(&paths.state_file)?;

    let result: Result<()> = async {
        let metadata = upstream::fetch_remote_metadata(&client, &config.dmg_url).await?;
        let previous_headers_fingerprint = state.remote_headers_fingerprint.clone();
        state.remote_headers_fingerprint = Some(metadata.headers_fingerprint.clone());
        state.last_successful_check_at = Some(Utc::now());

        if previous_headers_fingerprint.as_deref() == Some(metadata.headers_fingerprint.as_str())
            && state.dmg_sha256.is_some()
        {
            state.status = UpdateStatus::Idle;
            state.save(&paths.state_file)?;
            info!("upstream fingerprint unchanged; skipping download");
            return Ok(());
        }

        state.status = UpdateStatus::DownloadingDmg;
        state.save(&paths.state_file)?;

        let downloads_dir = config.workspace_root.join("downloads");
        let downloaded =
            upstream::download_dmg(&client, &config.dmg_url, &downloads_dir, Utc::now()).await?;

        if state.dmg_sha256.as_deref() == Some(downloaded.sha256.as_str()) {
            state.status = UpdateStatus::Idle;
            state.artifact_paths.dmg_path = Some(downloaded.path);
            state.save(&paths.state_file)?;
            info!("downloaded DMG hash matches current cached DMG; no update detected");
            return Ok(());
        }

        state.status = UpdateStatus::UpdateDetected;
        state.candidate_version = Some(downloaded.candidate_version);
        state.dmg_sha256 = Some(downloaded.sha256);
        state.artifact_paths.dmg_path = Some(downloaded.path.clone());
        state.save(&paths.state_file)?;

        let candidate_version = state
            .candidate_version
            .clone()
            .expect("candidate version should be set before local build");
        builder::build_update(config, state, paths, &candidate_version, &downloaded.path).await?;
        Ok(())
    }
    .await;

    if let Err(error) = result {
        state.mark_failed(error.to_string());
        state.save(&paths.state_file)?;
        return Err(error);
    }

    Ok(())
}
