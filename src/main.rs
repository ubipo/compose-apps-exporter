use clap::{parser::ValueSource, CommandFactory, FromArgMatches, Parser};
use directories::ProjectDirs;
use figment::{
    providers::{Env, Format, Serialized, Yaml},
    Figment,
};
use hyper::http::HeaderValue;
use hyper::service::{make_service_fn, service_fn};
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use indoc::indoc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::HashMap, net::IpAddr};
use std::{convert::Infallible, str::FromStr};
use std::{net::SocketAddr, path::Path};

static ENV_PREFIX: &str = "COMPOSE_APPS_EXPORTER_";

#[cfg(target_os = "macos")]
static SYSTEM_CONFIG_FILE_PATH: &str = "/usr/local/etc/";
#[cfg(target_os = "linux")]
static SYSTEM_CONFIG_FILE_PATH: &str = "/etc/";
#[cfg(target_os = "windows")]
static SYSTEM_CONFIG_FILE_PATH: &str = "C:\\ProgramData\\";

/// Prometheus metrics exporter for docker compose apps.
#[derive(Parser, Deserialize, Serialize, Debug)]
#[command(author, version, about, long_about = None)]
struct Config {
    /// Glob pattern for docker-compose.yml files or directories containing them
    #[arg(short, long, default_value = "/etc/compose-apps/*")]
    compose_configs_glob: Vec<String>,
    /// Port to listen on
    #[arg(short, long, default_value = "9179")]
    port: u16,
    /// Address to listen on
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,
}

struct ParsedConfig {
    pub compose_configs_glob: Vec<String>,
    pub port: u16,
    pub address: IpAddr,
}

impl TryFrom<Config> for ParsedConfig {
    type Error = Box<dyn std::error::Error>;

    fn try_from(config: Config) -> Result<Self, Self::Error> {
        let address = IpAddr::from_str(&config.address)?;
        Ok(ParsedConfig {
            compose_configs_glob: config.compose_configs_glob,
            port: config.port,
            address,
        })
    }
}

#[derive(Deserialize)]
struct ComposeService {
    container_name: String,
}

#[derive(Deserialize)]
struct ComposeConfig {
    name: String,
    services: HashMap<String, ComposeService>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Container {
    name: String,
    status: String,
    health: String,
}

fn config_paths_from_globs(
    config_path_globs: &[String],
) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
    let paths: Vec<_> = config_path_globs
        .iter()
        .map(|glob| glob::glob(glob))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Result<Vec<_>, _>>()?;
    let config_file_paths = paths
        .iter()
        .map(|path| {
            if path.is_dir() {
                path.join("docker-compose.yml")
            } else if path.is_file() {
                path.clone()
            } else {
                panic!("Invalid path: {}", path.display());
            }
        })
        .collect();
    return Ok(config_file_paths);
}

fn exec_docker_compose_cmd(
    config_path: impl AsRef<std::path::Path>,
    args: &[&str],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(config_path.as_ref())
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "'docker compose {}' failed with status code {}",
            args.join(" "),
            output.status
        )
        .into());
    }
    Ok(output.stdout)
}

fn read_compose_config(
    config_path: impl AsRef<std::path::Path>,
) -> Result<ComposeConfig, Box<dyn std::error::Error>> {
    let config = serde_yaml::from_slice(&exec_docker_compose_cmd(config_path, &["config"])?)?;
    Ok(config)
}

fn read_running_compose_containers(
    config_path: impl AsRef<std::path::Path>,
) -> Result<Vec<Container>, Box<dyn std::error::Error>> {
    let running_containers: Vec<Container> = serde_json::from_slice(&exec_docker_compose_cmd(
        config_path,
        &["ps", "--format", "json"],
    )?)?;
    Ok(running_containers)
}

fn config_and_containers_to_metrics(
    compose_config: ComposeConfig,
    running_containers: Vec<Container>,
) -> String {
    let service_names = compose_config.services.keys();
    let metrics = service_names.flat_map(|service_name| {
        let container_name = &compose_config.services[service_name].container_name;
        let container = running_containers
            .iter()
            .find(|container| container.name == *container_name);
        let status = match container {
            Some(container) => container.status.as_str().starts_with("Up") as u8,
            None => 0,
        };
        let health = match container {
            Some(container) => (container.health.as_str() == "healthy") as u8,
            None => 0,
        };
        vec![
            format!(
                "compose_service_up{{compose_name=\"{}\", compose_service=\"{}\"}} {}",
                compose_config.name, service_name, status
            ),
            format!(
                "compose_service_health{{compose_name=\"{}\", compose_service=\"{}\"}} {}",
                compose_config.name, service_name, health
            ),
        ]
    });
    return metrics.collect::<Vec<String>>().join("\n");
}

async fn get_metrics_for_configs_paths(
    config_paths: Vec<impl AsRef<std::path::Path>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let config_metrics_comment = indoc! {"
        # HELP compose_service_up Whether the docker compose services's status is 'Up' (as opposed to e.g. 'Restarting')
        # TYPE compose_service_up gauge
        # HELP compose_service_health Whether the docker compose services's health is 'healthy'
        # TYPE compose_service_health gauge
    "};
    let nbro_config_paths = config_paths.len();
    let config_metrics = config_paths
        .into_iter()
        .map(|config_path| {
            let config = read_compose_config(config_path.as_ref())?;
            let running_containers = read_running_compose_containers(config_path.as_ref())?;
            Ok(config_and_containers_to_metrics(config, running_containers))
        })
        .collect::<Result<Vec<String>, Box<dyn std::error::Error>>>()?
        .join("\n");
    let nbro_configs_metric = format!(
        indoc! {"
            # HELP compose_apps_nbro_configs Number of docker-compose apps
            # TYPE compose_apps_nbro_configs gauge
            compose_apps_nbro_configs {}
        "},
        nbro_config_paths
    );
    Ok(format!(
        "{}{}{}",
        config_metrics_comment, config_metrics, nbro_configs_metric
    ))
}

async fn get_metrics_for_config_globs(
    config_globs: &[String],
) -> Result<String, Box<dyn std::error::Error>> {
    let config_paths = config_paths_from_globs(config_globs)?;
    get_metrics_for_configs_paths(config_paths).await
}

async fn handle_request(
    compose_config_globs: Vec<String>,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let mut response = Response::new(Body::empty());

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            *response.status_mut() = StatusCode::PERMANENT_REDIRECT;
            response
                .headers_mut()
                .insert(header::LOCATION, HeaderValue::from_static("/metrics"));
        }
        (&Method::GET, "/metrics") => {
            let maybe_metrics = get_metrics_for_config_globs(&compose_config_globs).await;
            *response.body_mut() = match maybe_metrics {
                Ok(mut metrics) => {
                    metrics.push('\n');
                    Body::from(metrics)
                }
                Err(e) => {
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    eprintln!("Error while handling /metrics request: {}", e);
                    Body::from("Internal server error. Check logs for details.")
                }
            };
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };

    Ok(response)
}

fn get_config() -> Result<ParsedConfig, Box<dyn std::error::Error>> {
    let user_config_path = ProjectDirs::from("net", "pfiers", "compose-apps-exporter")
        .ok_or("Could not find user config directory")?
        .config_dir()
        .join("config.yaml");
    let system_config_path =
        Path::new(SYSTEM_CONFIG_FILE_PATH).join("compose-apps-exporter/config.yaml");

    let cli_command = Config::command();
    let cli_matches = cli_command
        .after_help(format!(
            indoc! {"
                From lowest to highest priority, configuration is loaded from:
                    - Default values
                    - User configuration file ({})
                    - System configuration file ({})
                    - Environment variables (prefixed with '{}')
                    - Command line arguments
            "},
            user_config_path.to_string_lossy(),
            system_config_path.to_string_lossy(),
            ENV_PREFIX
        ))
        .get_matches();
    let cli_args = Config::from_arg_matches(&cli_matches)?;
    let cli_args_without_defaults =
        serde_json::from_value::<Map<String, Value>>(serde_json::to_value(&cli_args)?)?
            .into_iter()
            .filter(|(k, _)| cli_matches.value_source(k) != Some(ValueSource::DefaultValue))
            .collect::<Map<String, Value>>();

    let config: Config = Figment::new()
        .merge(Yaml::file(user_config_path))
        .merge(Yaml::file(system_config_path))
        .merge(Env::prefixed(ENV_PREFIX))
        // Fill in defaults for the CLI args (though confusingly, the 'defaults'
        // here below refers to a figment profile, not a way to get default
        // values)
        .join(Serialized::defaults(cli_args))
        .merge(Serialized::defaults(cli_args_without_defaults))
        .extract::<Config>()?
        .into();

    let parsed_config: ParsedConfig = config.try_into()?;

    return Ok(parsed_config);
}

#[tokio::main]
async fn main() {
    let config = match get_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading config: \n{}", e);
            std::process::exit(1);
        }
    };
    let socket_address = SocketAddr::from((config.address, config.port));

    let make_svc = make_service_fn(move |_conn| {
        let compose_configs_glob = config.compose_configs_glob.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                handle_request(compose_configs_glob.clone(), req)
            }))
        }
    });

    let server = Server::bind(&socket_address).serve(make_svc);

    println!(
        "compose-apps-exporter listening on http://{}",
        socket_address
    );
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
        std::process::exit(1);
    }
}
