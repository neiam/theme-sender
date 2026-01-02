use clap::Parser;
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{info, debug, instrument};

#[derive(Debug, Parser)]
#[command(name = "theme-override")]
#[command(about = "Send custom theme overrides to theme-sender", long_about = None)]
struct Args {
    /// Custom theme string to send (e.g., "dark", "light", "high-contrast")
    #[arg(value_name = "THEME")]
    theme: Option<String>,
    
    /// Revert to automatic solar-based themes
    #[arg(short, long, conflicts_with = "theme")]
    revert: bool,
    
    #[command(flatten)]
    mqtt: MqttArgs,
}

#[derive(Debug, Parser, Clone)]
struct MqttArgs {
    #[arg(long, default_value = "localhost", env = "MQTT_HOST")]
    mqtt_host: String,

    #[arg(long, env = "MQTT_USERNAME")]
    mqtt_username: Option<String>,

    #[arg(long, env = "MQTT_PASSWORD")]
    mqtt_password: Option<String>,

    #[arg(long, default_value="neiam/sync/theme/override", env = "MQTT_OVERRIDE_TOPIC")]
    mqtt_override_topic: String,
    
    #[arg(long, default_value="neiam/sync/theme/revert", env = "MQTT_REVERT_TOPIC")]
    mqtt_revert_topic: String,
}

#[instrument]
fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    let args = Args::parse();
    debug!("Parsed arguments: {:?}", args);
    
    // Create MQTT client
    let create_opts = paho_mqtt::CreateOptionsBuilder::new()
        .server_uri(&args.mqtt.mqtt_host)
        .client_id("theme-override-cli")
        .finalize();
    
    let client = paho_mqtt::Client::new(create_opts)
        .context("Failed to create MQTT client")?;
    
    debug!("MQTT client created successfully");
    
    // Set up connection options
    let mut conn_opts_builder = paho_mqtt::ConnectOptionsBuilder::new();
    conn_opts_builder.keep_alive_interval(Duration::from_secs(20));
    
    if let (Some(username), Some(password)) = (&args.mqtt.mqtt_username, &args.mqtt.mqtt_password) {
        debug!("Using MQTT authentication with username: {}", username);
        conn_opts_builder.user_name(username).password(password);
    }
    
    let conn_opts = conn_opts_builder.finalize();
    
    
    info!("Connecting to MQTT broker at {}...", args.mqtt.mqtt_host);
    client.connect(conn_opts)
        .context("Failed to connect to MQTT broker")?;
    info!("Connected successfully");
    
    if args.revert {
        // Send revert message
        info!("🔄 Reverting to automatic solar-based themes");
        debug!("Publishing revert message to topic: {}", args.mqtt.mqtt_revert_topic);
        let msg = paho_mqtt::Message::new(&args.mqtt.mqtt_revert_topic, "revert", 1);
        client.publish(msg)
            .context("Failed to publish revert message")?;
        info!("✓ Revert message sent to {}", args.mqtt.mqtt_revert_topic);
    } else if let Some(theme) = args.theme {
        // Send custom theme override
        info!("🎭 Setting custom theme override: {}", theme);
        debug!("Publishing theme '{}' to topic: {}", theme, args.mqtt.mqtt_override_topic);
        let msg = paho_mqtt::Message::new(&args.mqtt.mqtt_override_topic, theme.clone(), 1);
        client.publish(msg)
            .context("Failed to publish override message")?;
        info!("✓ Custom theme '{}' sent to {}", theme, args.mqtt.mqtt_override_topic);
        info!("");
        info!("This theme will be active until the next solar event change.");
        info!("To revert to automatic themes immediately, run:");
        info!("  theme-override --revert");
    } else {
        eprintln!("Error: Must specify either a THEME or --revert");
        std::process::exit(1);
    }
    
    // Disconnect
    debug!("Disconnecting from MQTT broker");
    client.disconnect(None)
        .context("Failed to disconnect from MQTT broker")?;
    debug!("Disconnected successfully");
    
    Ok(())
}
