# Theme Sender

A smart MQTT-based theme publisher that automatically switches themes based on solar events (sunrise, sunset, etc.) with support for custom theme overrides.

## Features

- 🌅 **Solar-Based Themes**: Automatically calculates and publishes themes based on your location's solar events
- 🔄 **Periodic Publishing**: Regularly publishes the current theme (default: every 5 minutes)
- 🎭 **Custom Theme Override**: Send a custom theme via MQTT that will be published until the next solar event change
- 📍 **Automatic Geolocation**: Uses your IP address to determine location for solar calculations
- 🔧 **CLI Tool**: Convenient `theme-override` binary to send overrides and revert to automatic themes
- 🔐 **MQTT Authentication**: Supports username/password authentication
- 📊 **Structured Logging**: Configurable logging with trace, debug, info levels using `tracing`

## Theme Types

The following solar-based themes are automatically calculated:

- **Night** (`dark`) - Full night, stars visible
- **Astronomical Dawn** (`dark-dimmed`) - Faint light appears in sky
- **Nautical Dawn** (`dark-soft`) - Horizon becomes visible
- **Civil Dawn** (`light-soft`) - Enough light for outdoor activities
- **Sunrise** (`light`) - Sun breaks the horizon
- **Day** (`light`) - Full daylight
- **Civil Dusk** (`light-soft`) - Sun below horizon, still light out
- **Nautical Dusk** (`dark-soft`) - Darker, horizon still visible
- **Astronomical Dusk** (`dark-dimmed`) - Fading light in sky

## Configuration

### Command Line Arguments

```bash
theme-sender \
  --mqtt-host <HOST> \
  --mqtt-username <USERNAME> \
  --mqtt-password <PASSWORD> \
  --mqtt-topic <TOPIC> \
  --mqtt-override-topic <OVERRIDE_TOPIC> \
  --publish-interval-secs <SECONDS>
```

### Environment Variables

All configuration can be set via environment variables:

- `MQTT_HOST` - MQTT broker host (default: `localhost`)
- `MQTT_USERNAME` - MQTT username (optional)
- `MQTT_PASSWORD` - MQTT password (optional)
- `MQTT_TOPIC` - Topic to publish themes to (default: `neiam/sync/theme`)
- `MQTT_OVERRIDE_TOPIC` - Topic to receive custom theme overrides (default: `neiam/sync/theme/override`)
- `MQTT_REVERT_TOPIC` - Topic to receive revert commands (default: `neiam/sync/theme/revert`)
- `PUBLISH_INTERVAL_SECS` - How often to publish the theme in seconds (default: `300`)

### Logging Configuration

Both binaries use the `tracing` library for structured logging. Configure via the `RUST_LOG` environment variable:

```bash
# Show all logs (default: info level)
RUST_LOG=theme_sender=info cargo run

# Show debug logs for more detail
RUST_LOG=theme_sender=debug cargo run

# Show trace logs for maximum verbosity
RUST_LOG=theme_sender=trace cargo run
```

## Usage

### Basic Usage

```bash
cargo run
```

### Custom Configuration

```bash
MQTT_HOST=mqtt.example.com \
MQTT_USERNAME=user \
MQTT_PASSWORD=pass \
PUBLISH_INTERVAL_SECS=60 \
cargo run
```

### Custom Theme Override

To temporarily override the automatic solar theme, publish a custom theme to the override topic:

```bash
# Using mosquitto_pub
mosquitto_pub -h localhost -t "neiam/sync/theme/override" -m "my-custom-theme"

# Using another MQTT client
# Send the theme string as the payload to the override topic
```

The custom theme will be published periodically until the next solar event causes a theme change, at which point it will automatically clear the override and return to solar-based themes.

## Published Message Format

The theme is published as JSON:

```json
{
  "theme": "light",
  "data": "2025-12-29T04:30:00Z"
}
```

## How It Works

1. **Startup**: 
   - Fetches your location based on IP address
   - Calculates today's solar events
   - Publishes the current theme immediately

2. **Main Loop**:
   - Every N seconds (configurable):
     - Checks for custom theme overrides
     - Determines the current solar theme
     - Publishes the appropriate theme (custom or solar)
     - Clears custom overrides when solar theme changes

3. **Override Listener**:
   - Runs in background
   - Listens for custom theme messages
   - Queues them for the main loop to process

## Building

```bash
cargo build --release
```
