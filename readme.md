# Compose Apps Exporter 🐳📤🔥

Prometheus exporter for docker compose apps

On every scrape, the exporter will read all [docker compose files](https://docs.docker.com/compose/compose-file/compose-file-v3/) in the given
paths and, for every service of every app, will export the following metrics:
- `compose_service_up{compose_app="my-app", compose_service="my-service"}`  
(1 if the service status is 'Up', 0 otherwise)
- `compose_service_health{compose_app="my-app", compose_service="my-service"}`  
(1 if the service health is 'healthy', 0 otherwise)

Both of these metrics will have a value of `1` if the service is up and healthy,
and `0` otherwise.

Additionally, the exporter will export a `compose_apps_nbro_configs` metric with
the number of compose files it has read.

## Usage

### Local

```bash
compose-apps-exporter
```

...or with custom configuration:

```bash
compose-apps-exporter --port 9200 --address "127.24.0.1" --compose-configs-glob "/etc/my-own-path-to-compose-apps/**/non-standard.yaml"
```

### Docker

```bash
docker run -d -p 9179:9179 -v /path/to/compose/apps:/etc/compose-apps:ro --name compose-apps-exporter compose-apps-exporter
```

## Configuration

From lowest to highest priority, configuration is loaded from:
  - Default values
  - User configuration file (OS specific, see '-h' for details)
  - System configuration file (OS specific, see '-h' for details, `/etc/compose-apps-exporter/config.yaml` on Linux)
  - Environment variables (prefixed with 'COMPOSE_APPS_EXPORTER_')
  - Command line arguments

### Configuration File Format

```yaml
compose_configs_glob:
  - "/etc/my-own-path-to-compose-apps/**/non-standard.yaml"
port: 8854
address: "127.24.0.1"
```