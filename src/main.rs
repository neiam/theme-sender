use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration as StdDuration;
use sunrise::{Coordinates, DawnType, SolarDay, SolarEvent};
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument};
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    info!("Starting theme sender...");
    info!("MQTT Host: {}", args.mqtt.mqtt_host);
    info!("MQTT Topic: {}", args.mqtt.mqtt_topic);
    info!("MQTT Override Topic: {}", args.mqtt.mqtt_override_topic);
    info!("MQTT Revert Topic: {}", args.mqtt.mqtt_revert_topic);
    debug!("MQTT Username: {:?}", args.mqtt.mqtt_username);

    // Get location
    info!("Fetching geolocation...");
    let location = get_location().await?;
    info!(
        "Location: lat={:.4}, lon={:.4}",
        location.latitude, location.longitude
    );

    let coordinates =
        Coordinates::new(location.latitude, location.longitude).context("Invalid coordinates")?;

    // Configure how often to publish the theme (in seconds)
    let publish_interval = args.publish_interval_secs;
    info!("Publishing theme every {} seconds", publish_interval);

    // Create a channel for receiving custom theme overrides
    let (override_tx, mut override_rx) = mpsc::channel::<OverrideMessage>(10);

    // Spawn MQTT listener task for custom theme overrides
    info!("Spawning MQTT listener task...");
    let mqtt_args = args.mqtt.clone();
    tokio::spawn(async move {
        if let Err(e) = mqtt_listener(mqtt_args, override_tx).await {
            error!("MQTT listener error: {}", e);
        }
    });

    // Publish current theme immediately on startup
    let mut last_published_theme = {
        debug!("Calculating initial theme based on solar events");
        let now = Utc::now();
        let date = now.date_naive();
        let solar_day = SolarDay::new(coordinates, date);

        let mut events = [
            (
                ThemeType::AstronomicalDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Astronomical)),
            ),
            (
                ThemeType::NauticalDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Nautical)),
            ),
            (
                ThemeType::CivilDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Civil)),
            ),
            (
                ThemeType::Sunrise,
                solar_day.event_time(SolarEvent::Sunrise),
            ),
            (ThemeType::Day, solar_day.event_time(SolarEvent::Sunset)),
            (
                ThemeType::CivilDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Civil)),
            ),
            (
                ThemeType::NauticalDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Nautical)),
            ),
            (
                ThemeType::AstronomicalDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Astronomical)),
            ),
        ];

        events.sort_by_key(|(_, time)| *time);

        // Find current theme (the last event that has passed)
        let current_theme = events
            .iter()
            .rev()
            .find(|(_, time)| *time <= now)
            .map(|(theme, _)| theme.clone())
            .unwrap_or(ThemeType::Night);

        info!(
            "🌟 Publishing current theme on startup: {:?}",
            current_theme
        );
        send_theme_update(&args.mqtt, &current_theme).await?;
        Some(current_theme.clone())
    };

    let mut custom_override: Option<String> = None;
    let mut last_solar_theme: Option<ThemeType> = None;
    let _immediate_check = false; // Flag to skip sleep and check immediately

    loop {
        debug!("Starting new publish cycle");
        let now = Utc::now();
        let date = now.date_naive();

        // Calculate all solar events for today
        let solar_day = SolarDay::new(coordinates, date);

        let mut events = vec![
            (
                ThemeType::AstronomicalDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Astronomical)),
            ),
            (
                ThemeType::NauticalDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Nautical)),
            ),
            (
                ThemeType::CivilDawn,
                solar_day.event_time(SolarEvent::Dawn(DawnType::Civil)),
            ),
            (
                ThemeType::Sunrise,
                solar_day.event_time(SolarEvent::Sunrise),
            ),
            (ThemeType::Day, solar_day.event_time(SolarEvent::Sunset)),
            (
                ThemeType::CivilDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Civil)),
            ),
            (
                ThemeType::NauticalDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Nautical)),
            ),
            (
                ThemeType::AstronomicalDusk,
                solar_day.event_time(SolarEvent::Dusk(DawnType::Astronomical)),
            ),
            (
                ThemeType::Night,
                DateTime::from_timestamp(now.timestamp() + 86400, 0)
                    .unwrap()
                    .with_timezone(&Utc),
            ), // Next day midnight
        ];

        // Sort by time
        events.sort_by_key(|(_, time)| *time);

        // Print today's schedule
        info!("Today's schedule:");
        for (theme, time) in &events {
            if time.date_naive() == date {
                info!("  {} - {:?}", time.format("%H:%M:%S"), theme);
            }
        }

        // Check for custom theme override messages
        // Process all pending messages before continuing
        let mut theme_changed = false;
        loop {
            match override_rx.try_recv() {
                Ok(msg) => {
                    match msg {
                        OverrideMessage::SetTheme(theme) => {
                            info!("🎭 Received custom theme override: {}", theme);
                            debug!("Setting custom_override to: {}", theme);

                            // Check if this is actually a change
                            if custom_override.as_ref() != Some(&theme) {
                                custom_override = Some(theme.clone());
                                theme_changed = true;

                                // Publish immediately
                                let new_theme = ThemeType::Custom(theme);
                                info!("🎨 Publishing new custom theme immediately");
                                send_theme_update(&args.mqtt, &new_theme).await?;
                                last_published_theme = Some(new_theme);
                            } else {
                                debug!("Custom theme unchanged, skipping republish");
                            }
                        }
                        OverrideMessage::Revert => {
                            info!("🔄 Received revert message, clearing custom override");
                            debug!("Clearing custom_override");

                            if custom_override.is_some() {
                                custom_override = None;
                                theme_changed = true;

                                // Publish current solar theme immediately
                                let solar_theme = events
                                    .iter()
                                    .rev()
                                    .find(|(_, time)| *time <= now)
                                    .map(|(theme, _)| theme.clone())
                                    .unwrap_or(ThemeType::Night);
                                info!("🎨 Publishing solar theme immediately: {:?}", solar_theme);
                                send_theme_update(&args.mqtt, &solar_theme).await?;
                                last_published_theme = Some(solar_theme);
                            }
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // No more messages, continue
                    break;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    error!("Override channel disconnected! MQTT listener may have crashed.");
                    break;
                }
            }
        }

        // Determine what theme to use
        let solar_theme = events
            .iter()
            .rev()
            .find(|(_, time)| *time <= now)
            .map(|(theme, _)| theme.clone())
            .unwrap_or(ThemeType::Night);
        debug!("Current solar theme: {:?}", solar_theme);

        // Check if solar theme has changed (which would clear the override)
        if custom_override.is_some()
            && let Some(ref last_solar) = last_solar_theme
            && last_solar != &solar_theme
        {
            info!(
                "☀️  Solar theme changed from {:?} to {:?}, clearing custom override",
                last_solar, solar_theme
            );
            custom_override = None;
        }

        // Update last solar theme
        if last_solar_theme.as_ref() != Some(&solar_theme) {
            debug!(
                "Solar theme updated from {:?} to {:?}",
                last_solar_theme, solar_theme
            );
            last_solar_theme = Some(solar_theme.clone());
        }

        let current_theme = if let Some(ref custom) = custom_override {
            debug!("Using custom theme: {}", custom);
            ThemeType::Custom(custom.clone())
        } else {
            debug!("Using solar theme: {:?}", solar_theme);
            solar_theme
        };

        // Publish if theme has changed or it's time for periodic update
        if last_published_theme.as_ref() != Some(&current_theme) {
            info!("🎨 Theme changed to {:?}", current_theme);
            send_theme_update(&args.mqtt, &current_theme).await?;
            last_published_theme = Some(current_theme.clone());
        } else {
            info!("♻️  Republishing current theme: {:?}", current_theme);
            send_theme_update(&args.mqtt, &current_theme).await?;
        }

        // Wait for the configured interval before next check
        // If we just published due to an override change, skip the sleep
        // and immediately loop to check for more messages
        if theme_changed {
            debug!(
                "Theme was changed by override/revert, continuing immediately to check for more messages"
            );
            // Don't sleep, just continue the loop
        } else {
            debug!("Waiting {} seconds until next check...", publish_interval);
            tokio::time::sleep(StdDuration::from_secs(publish_interval)).await;
        }
    }
}

#[derive(Debug, Clone)]
enum OverrideMessage {
    SetTheme(String),
    Revert,
}

#[instrument(skip(override_tx))]
async fn mqtt_listener(
    args: ThemeMqttArgs,
    override_tx: mpsc::Sender<OverrideMessage>,
) -> Result<()> {
    info!("Starting MQTT listener for custom theme overrides");

    // Run MQTT operations in a blocking task since paho-mqtt is not async
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut reconnect_delay = 1u64; // Start with 1 second
        const MAX_RECONNECT_DELAY: u64 = 60; // Cap at 60 seconds

        loop {
            debug!("Creating MQTT client for listener");
            // Create MQTT client
            let create_opts = paho_mqtt::CreateOptionsBuilder::new()
                .server_uri(&args.mqtt_host)
                .client_id("theme-sender-listener")
                .finalize();

            let client = match paho_mqtt::Client::new(create_opts) {
                Ok(c) => c,
                Err(e) => {
                    error!("Failed to create MQTT client: {}", e);
                    std::thread::sleep(StdDuration::from_secs(reconnect_delay));
                    reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                    continue;
                }
            };

            // Set up connection options with auto-reconnect
            debug!("Configuring MQTT connection for listener");
            let mut conn_opts_builder = paho_mqtt::ConnectOptionsBuilder::new();
            conn_opts_builder
                .keep_alive_interval(StdDuration::from_secs(20))
                .clean_session(false)
                .automatic_reconnect(StdDuration::from_secs(1), StdDuration::from_secs(60));

            if let (Some(username), Some(password)) = (&args.mqtt_username, &args.mqtt_password) {
                debug!("Using MQTT authentication for listener");
                conn_opts_builder.user_name(username).password(password);
            }

            let conn_opts = conn_opts_builder.finalize();

            // Start consumer before connecting
            let rx = client.start_consuming();

            // Connect
            info!("Connecting MQTT listener to broker");
            if let Err(e) = client.connect(conn_opts) {
                error!("Failed to connect to MQTT broker: {}", e);
                std::thread::sleep(StdDuration::from_secs(reconnect_delay));
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            // Subscribe to override topic
            debug!(
                "Subscribing to override topic: {}",
                args.mqtt_override_topic
            );
            if let Err(e) = client.subscribe(&args.mqtt_override_topic, 1) {
                error!("Failed to subscribe to override topic: {}", e);
                std::thread::sleep(StdDuration::from_secs(reconnect_delay));
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            // Subscribe to revert topic
            debug!("Subscribing to revert topic: {}", args.mqtt_revert_topic);
            if let Err(e) = client.subscribe(&args.mqtt_revert_topic, 1) {
                error!("Failed to subscribe to revert topic: {}", e);
                std::thread::sleep(StdDuration::from_secs(reconnect_delay));
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            info!(
                "✓ Subscribed to {} and {}",
                args.mqtt_override_topic, args.mqtt_revert_topic
            );

            // Reset reconnect delay on successful connection
            reconnect_delay = 1;

            // Listen for messages
            info!("MQTT listener ready, waiting for messages");
            let connection_lost;
            loop {
                // Use recv with timeout to avoid blocking indefinitely
                match rx.recv_timeout(StdDuration::from_millis(100)) {
                    Ok(Some(msg)) => {
                        let topic = msg.topic();
                        let payload = String::from_utf8_lossy(msg.payload()).to_string();

                        debug!(
                            "Received MQTT message on topic '{}' with payload '{}'",
                            topic, payload
                        );

                        let override_msg = if topic == args.mqtt_revert_topic {
                            OverrideMessage::Revert
                        } else {
                            OverrideMessage::SetTheme(payload.clone())
                        };

                        debug!("Parsed as: {:?}", override_msg);

                        // Use blocking_send since we're in a blocking context
                        if let Err(e) = override_tx.blocking_send(override_msg) {
                            error!("Failed to send override message to main loop: {}", e);
                            return Err(anyhow::anyhow!("Override channel closed"));
                        }
                        debug!("Successfully forwarded message to main loop");
                    }
                    Ok(None) => {
                        debug!("Connection lost, attempting to reconnect...");
                        connection_lost = true;
                        break;
                    }
                    Err(_) => {
                        // Timeout is normal, check connection status
                        if !client.is_connected() {
                            error!("MQTT connection lost, attempting to reconnect...");
                            connection_lost = true;
                            break;
                        }
                    }
                }
            }

            if connection_lost {
                error!(
                    "Connection lost, reconnecting in {} seconds...",
                    reconnect_delay
                );
                std::thread::sleep(StdDuration::from_secs(reconnect_delay));
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            break;
        }

        Ok(())
    })
    .await
    .context("MQTT listener task panicked")?
    .context("MQTT listener error")?;

    Ok(())
}

#[instrument(skip(args))]
async fn send_theme_update(args: &ThemeMqttArgs, theme: &ThemeType) -> Result<()> {
    let payload = ThemePayload {
        theme: theme.to_theme_string(),
        data: Utc::now(),
    };

    info!(
        "🎨 Sending theme update: {} ({})",
        payload.theme,
        theme.to_description()
    );
    debug!("Theme payload: {:?}", payload);

    let payload_json = serde_json::to_string(&payload)?;

    // Retry logic with exponential backoff
    let mut retry_delay = 1u64;
    const MAX_RETRY_DELAY: u64 = 30;
    const MAX_RETRIES: u32 = 5;

    for attempt in 1..=MAX_RETRIES {
        match try_send_mqtt(args, &payload_json, attempt).await {
            Ok(()) => {
                info!("✓ Theme update sent successfully");
                return Ok(());
            }
            Err(e) if attempt < MAX_RETRIES => {
                error!(
                    "Failed to send theme update (attempt {}/{}): {}",
                    attempt, MAX_RETRIES, e
                );
                debug!("Retrying in {} seconds...", retry_delay);
                tokio::time::sleep(StdDuration::from_secs(retry_delay)).await;
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
            Err(e) => {
                error!(
                    "Failed to send theme update after {} attempts: {}",
                    MAX_RETRIES, e
                );
                return Err(e);
            }
        }
    }

    Err(anyhow::anyhow!(
        "Failed to send theme update after all retries"
    ))
}

async fn try_send_mqtt(args: &ThemeMqttArgs, payload_json: &str, attempt: u32) -> Result<()> {
    debug!("Attempting to send MQTT message (attempt {})", attempt);

    // Create MQTT client
    let create_opts = paho_mqtt::CreateOptionsBuilder::new()
        .server_uri(&args.mqtt_host)
        .client_id("theme-sender")
        .finalize();

    let client = paho_mqtt::Client::new(create_opts).context("Failed to create MQTT client")?;

    // Set up connection options with automatic reconnect
    let mut conn_opts_builder = paho_mqtt::ConnectOptionsBuilder::new();
    conn_opts_builder
        .keep_alive_interval(StdDuration::from_secs(20))
        .automatic_reconnect(StdDuration::from_secs(1), StdDuration::from_secs(30));

    if let (Some(username), Some(password)) = (&args.mqtt_username, &args.mqtt_password) {
        debug!("Using MQTT authentication");
        conn_opts_builder.user_name(username).password(password);
    }

    let conn_opts = conn_opts_builder.finalize();

    // Connect
    client
        .connect(conn_opts)
        .context("Failed to connect to MQTT broker")?;

    // Publish message
    debug!("Publishing to topic {}: {}", args.mqtt_topic, payload_json);
    let msg = paho_mqtt::Message::new(&args.mqtt_topic, payload_json, 1);
    client.publish(msg).context("Failed to publish message")?;

    // Disconnect
    debug!("Disconnecting from MQTT broker");
    client
        .disconnect(None)
        .context("Failed to disconnect from MQTT broker")?;

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum ThemeType {
    Night,
    AstronomicalDawn,
    NauticalDawn,
    CivilDawn,
    Sunrise,
    Day,
    CivilDusk,
    NauticalDusk,
    AstronomicalDusk,
    Custom(String),
}

impl ThemeType {
    fn to_theme_string(&self) -> String {
        match self {
            ThemeType::Night => "dark".to_string(),
            ThemeType::AstronomicalDawn => "dark-dimmed".to_string(),
            ThemeType::NauticalDawn => "dark-soft".to_string(),
            ThemeType::CivilDawn => "light-soft".to_string(),
            ThemeType::Sunrise => "light".to_string(),
            ThemeType::Day => "light".to_string(),
            ThemeType::CivilDusk => "light-soft".to_string(),
            ThemeType::NauticalDusk => "dark-soft".to_string(),
            ThemeType::AstronomicalDusk => "dark-dimmed".to_string(),
            ThemeType::Custom(theme) => theme.clone(),
        }
    }

    fn to_description(&self) -> &'static str {
        match self {
            ThemeType::Night => "Full night - stars visible",
            ThemeType::AstronomicalDawn => "Astronomical dawn - faint light appears in sky",
            ThemeType::NauticalDawn => "Nautical dawn - horizon becomes visible",
            ThemeType::CivilDawn => "Civil dawn - enough light for outdoor activities",
            ThemeType::Sunrise => "Sunrise - sun breaks the horizon",
            ThemeType::Day => "Full daylight",
            ThemeType::CivilDusk => "Civil dusk - sun below horizon, still light out",
            ThemeType::NauticalDusk => "Nautical dusk - darker, horizon still visible",
            ThemeType::AstronomicalDusk => "Astronomical dusk - fading light in sky",
            ThemeType::Custom(_theme) => "Custom theme override",
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct ThemePayload {
    theme: String,
    data: DateTime<Utc>,
}

#[derive(Debug, Parser)]
struct Args {
    #[command(flatten)]
    mqtt: ThemeMqttArgs,

    #[arg(long, default_value = "300", env = "PUBLISH_INTERVAL_SECS")]
    publish_interval_secs: u64,
}

#[derive(Debug, Parser, Clone)]
struct ThemeMqttArgs {
    #[arg(long, default_value = "localhost", env = "MQTT_HOST")]
    mqtt_host: String,

    #[arg(long, env = "MQTT_USERNAME")]
    mqtt_username: Option<String>,

    #[arg(long, env = "MQTT_PASSWORD")]
    mqtt_password: Option<String>,

    #[arg(long, default_value = "neiam/sync/theme", env = "MQTT_TOPIC")]
    mqtt_topic: String,

    #[arg(
        long,
        default_value = "neiam/sync/theme/override",
        env = "MQTT_OVERRIDE_TOPIC"
    )]
    mqtt_override_topic: String,

    #[arg(
        long,
        default_value = "neiam/sync/theme/revert",
        env = "MQTT_REVERT_TOPIC"
    )]
    mqtt_revert_topic: String,
}

// Geolocation API integration
#[derive(Debug, Deserialize)]
struct IpApiResponse {
    lat: f64,
    lon: f64,
}

#[derive(Debug, Clone, Copy)]
struct Location {
    latitude: f64,
    longitude: f64,
}

#[instrument]
async fn get_location() -> Result<Location> {
    // Use ip-api.com to get location based on IP
    debug!("Fetching location from ip-api.com");
    let response: IpApiResponse = reqwest::get("http://ip-api.com/json/?fields=lat,lon")
        .await
        .context("Failed to fetch geolocation")?
        .json()
        .await
        .context("Failed to parse geolocation response")?;
    debug!(
        "Received location: lat={}, lon={}",
        response.lat, response.lon
    );

    Ok(Location {
        latitude: response.lat,
        longitude: response.lon,
    })
}
