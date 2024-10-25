use regex::Regex;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::IpAddr,
    process::Stdio,
};
use tokio::{net::TcpStream, process::Command};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerConfigError {
    #[error("Failed to parse config file: {0}")]
    ParseError(String),

    #[error("Found undefined dependency: {0}")]
    UndefinedDependency(String),

    #[error("Found circular dependency: {0}")]
    CircularDependency(String),

    #[error("Misconfigured healthcheck: {0}")]
    BadHealthCheckDefinition(String),
}

fn default_duration() -> std::time::Duration {
    std::time::Duration::from_secs(10)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthCheck {
    Http {
        url: String,
        status: Option<u16>,
        #[serde(default = "default_duration", with = "humantime_serde")]
        retry: std::time::Duration,
        #[serde(default, with = "serde_regex")]
        regex: Option<Regex>,
    },
    Port {
        ip: String,
        port: u16,
        #[serde(default = "default_duration", with = "humantime_serde")]
        retry: std::time::Duration,
    },
    Shell {
        command: String,
        #[serde(default = "default_duration", with = "humantime_serde")]
        retry: std::time::Duration,
        status: Option<i32>,
        #[serde(default, with = "serde_regex")]
        regex: Option<Regex>,
    },
}

#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    pub name: String,
    pub mac: String,
    pub interface: String,
    #[serde(default)]
    pub vlan: Option<u16>,

    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub check: Vec<HealthCheck>,
}

fn map_server_names(servers: &[Server]) -> HashMap<String, &Server> {
    servers.iter().map(|s| (s.name.clone(), s)).collect()
}

fn determine_wakeup_order(servers: &[Server]) -> Result<Vec<Server>, ServerConfigError> {
    let server_from_name = map_server_names(servers);

    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    let mut sorted = Vec::new();

    for server in servers {
        if !visited.contains(&server.name) {
            depth_first_search(
                server,
                &server_from_name,
                &mut visited,
                &mut visiting,
                &mut sorted,
            )?;
        }
    }

    let servers_in_order: Vec<Server> = sorted
        .iter()
        .map(|name| *server_from_name.get(name).unwrap())
        .cloned()
        .collect();
    Ok(servers_in_order)
}

fn depth_first_search(
    server: &Server,
    server_from_name: &HashMap<String, &Server>,
    visited: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
    sorted: &mut Vec<String>,
) -> Result<(), ServerConfigError> {
    if visiting.contains(&server.name) {
        return Err(ServerConfigError::CircularDependency(server.name.clone()));
    }

    if visited.contains(&server.name) {
        return Ok(());
    }

    visiting.insert(server.name.clone());

    for dep in &server.depends {
        let dep_server = server_from_name
            .get(dep)
            .ok_or_else(|| ServerConfigError::UndefinedDependency(dep.clone()))?;
        depth_first_search(dep_server, server_from_name, visited, visiting, sorted)?;
    }

    visiting.remove(&server.name);
    visited.insert(server.name.clone());

    sorted.push(server.name.clone());

    Ok(())
}

fn validate_health_check(healthcheck: &HealthCheck) -> Result<(), ServerConfigError> {
    match healthcheck {
        HealthCheck::Http {
            url: _,
            status,
            regex,
            retry: _,
        } => {
            if status.is_none() && regex.is_none() {
                return Err(ServerConfigError::BadHealthCheckDefinition("HTTP health check requires an HTTP status code to match and/or a Regex to match in the response".into()));
            }
        }
        HealthCheck::Port {
            ip,
            port: _,
            retry: _,
        } => {
            if ip.parse::<IpAddr>().is_err() {
                return Err(ServerConfigError::BadHealthCheckDefinition(
                    "Port check requires a valid IP address".into(),
                ));
            }
        }
        HealthCheck::Shell {
            command: _,
            status,
            regex,
            retry: _,
        } => {
            if status.is_none() && regex.is_none() {
                return Err(ServerConfigError::BadHealthCheckDefinition("Health check via shell command requires an return code to match and/or a Regex to match in the standard output".into()));
            }
        }
    }

    Ok(())
}

pub fn parse_server_dependencies(file_path: &str) -> Result<Vec<Server>, ServerConfigError> {
    let yaml_content =
        fs::read_to_string(file_path).map_err(|e| ServerConfigError::ParseError(e.to_string()))?;

    let servers: Vec<Server> = serde_yaml_ng::from_str(&yaml_content)
        .map_err(|e| ServerConfigError::ParseError(e.to_string()))?;

    for server in &servers {
        for healthcheck in &server.check {
            validate_health_check(healthcheck)?;
        }
    }

    // Apply topological sort to determine order to wake the servers
    // check for circular and undefined servers along the way
    let sorted = determine_wakeup_order(&servers)?;

    Ok(sorted)
}

async fn http_health_check(
    url: &str,
    expected_status: Option<u16>,
    payload_regex: Option<Regex>,
) -> bool {
    if let Ok(response) = reqwest::get(url).await {
        if let Some(status) = expected_status {
            println!("Check for status");
            if response.status().as_u16() != status {
                return false;
            }
            println!("Status matches");
        }
        if let Some(regex) = payload_regex {
            println!("Check for regex: {}", regex);
            if let Ok(body) = response.text().await {
                if regex.is_match(&body) {
                    println!("regex {} matches", regex);
                    return true;
                }
            };
            return false;
        }
        return true;
    };
    false
}

async fn port_health_check(ip: &str, port: u16) -> bool {
    let address = format!("{}:{}", ip, port);
    return TcpStream::connect(address).await.is_ok();
}

async fn shell_health_check(
    command: &str,
    expected_status: Option<i32>,
    payload_regex: Option<Regex>,
) -> bool {
    let result = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    if let Ok(output) = result {
        if let Some(status) = expected_status {
            if output.status.code() != Some(status) {
                return false;
            }
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(regex) = payload_regex {
            if !regex.is_match(&stdout) {
                return false;
            }
        }
        return true;
    };
    false
}

// TODO: Find a better way to handle this, it's really ugly
pub async fn check_wait(check: HealthCheck) -> () {
    let retry = match check {
        HealthCheck::Http {
            url: _,
            status: _,
            retry,
            regex: _,
        } => retry,
        HealthCheck::Port {
            ip: _,
            port: _,
            retry,
        } => retry,
        HealthCheck::Shell {
            command: _,
            retry,
            status: _,
            regex: _,
        } => retry,
    };
    tokio::time::sleep(retry).await
}

pub async fn check_health(check: HealthCheck) -> bool {
    match check {
        HealthCheck::Http {
            url,
            status,
            regex,
            retry: _,
        } => http_health_check(&url, status, regex).await,
        HealthCheck::Port { ip, port, retry: _ } => port_health_check(&ip, port).await,
        HealthCheck::Shell {
            command,
            status,
            regex,
            retry: _,
        } => shell_health_check(&command, status, regex).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_dependencies() {
        let yaml_data = r#"
        - name: "server1"
          mac: "00:11:22:33:44:55"
          interface: "eth0"
          vlan: 100
          depends:
            - "server2"
          check: []

        - name: "server2"
          mac: "66:77:88:99:AA:BB"
          interface: "eth0"
          vlan: 100
          depends:
            - "server3"
          check: []

        - name: "server3"
          mac: "AA:BB:CC:DD:EE:FF"
          interface: "eth1"
          vlan: 200
          depends:
            - "server4"
          check: []

        - name: "server4"
          mac: "FF:EE:DD:CC:BB:AA"
          interface: "eth1"
          vlan: 200
          depends:
            - "server1"
          check: []
        "#;

        let servers: Vec<Server> =
            serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");

        let result = determine_wakeup_order(&servers);
        match result {
            Err(ServerConfigError::CircularDependency(circular_server)) => {
                assert_eq!(circular_server, "server1".to_string());
            }
            _ => panic!("Expected a circular dependency error"),
        }
    }

    #[test]
    fn test_no_circular_dependencies_with() {
        let yaml_data = r#"
        - name: "server1"
          mac: "00:11:22:33:44:55"
          interface: "eth0"
          vlan: 100
          depends:
            - "server2"
          check: []

        - name: "server2"
          mac: "66:77:88:99:AA:BB"
          interface: "eth0"
          vlan: 100
          depends:
            - "server3"
          check: []

        - name: "server3"
          mac: "AA:BB:CC:DD:EE:FF"
          interface: "eth1"
          vlan: 200
          depends: []
          check: []
        "#;

        // Deserialize the YAML string into a ServerDependencyConfig
        let servers: Vec<Server> =
            serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");

        // Call the validate_dependencies function and check if it passes without errors.
        let result = determine_wakeup_order(&servers);

        // We expect no errors, meaning no circular dependencies exist.
        assert!(result.is_ok(), "Expected no circular dependencies");
    }

    #[test]
    fn test_invalid_http_check() {
        let yaml_data = r#"
        name: "server1"
        mac: "00:11:22:33:44:55"
        interface: "eth0"
        vlan: 100
        depends: []
        check:
          - type: http
            url: "http://example.com"
        "#;

        let server: Server = serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");
        let result = validate_health_check(&server.check[0]);
        assert!(matches!(
            result,
            Err(ServerConfigError::BadHealthCheckDefinition(_))
        ));
    }

    #[test]
    fn test_invalid_shell_check() {
        let yaml_data = r#"
        name: "server1"
        mac: "00:11:22:33:44:55"
        interface: "eth0"
        vlan: 100
        depends: []
        check:
          - type: shell
            command: curl something 
            retry: 2 minutes
        "#;

        let server: Server = serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");
        let result = validate_health_check(&server.check[0]);
        assert!(matches!(
            result,
            Err(ServerConfigError::BadHealthCheckDefinition(_))
        ));
    }

    #[test]
    fn test_invalid_port_check() {
        let yaml_data = r#"
        name: "server1"
        mac: "00:11:22:33:44:55"
        interface: "eth0"
        vlan: 100
        depends: []
        check:
          - type: port
            ip: "invalid_ip"   # Invalid IP address
            port: 80
        "#;

        let server: Server = serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");
        let result = validate_health_check(&server.check[0]);

        assert!(matches!(
            result,
            Err(ServerConfigError::BadHealthCheckDefinition(_))
        ));
    }

    #[test]
    fn test_valid_health_checks() {
        let yaml_data = r#"
        name: "server1"
        mac: "00:11:22:33:44:55"
        interface: "eth0"
        vlan: 100
        depends: []
        check:
          - type: http
            url: "http://example.com"
            status: 200          # Valid: status is provided
            regex: ~
          - type: port
            ip: "192.168.1.1"    # Valid IP
            port: 80
          - type: shell
            command: "echo Hello"
            status: ~            # Valid: regex is provided
            regex: "Hello"
        "#;

        let server: Server = serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");
        for healthcheck in &server.check {
            let result = validate_health_check(&healthcheck);
            assert!(result.is_ok())
        }
    }

    #[test]
    fn test_determine_wakeup_order() {
        // Define the YAML string for servers with dependencies
        let yaml_data = r#"
        - name: "server_a"
          mac: "00:11:22:33:44:55"
          interface: "eth0"
          depends:
            - "server_b"
            - "server_c"

        - name: "server_b"
          mac: "11:22:33:44:55:66"
          interface: "eth0"
          depends:
            - "server_c"

        - name: "server_c"
          mac: "22:33:44:55:66:77"
          interface: "eth0"
        "#;

        // Parse the YAML string into the expected structure
        let servers: Vec<Server> =
            serde_yaml_ng::from_str(yaml_data).expect("Failed to parse YAML");

        // Expected topologically sorted order (server_c first, then server_b, then server_a)
        let expected_order = vec!["server_c", "server_b", "server_a"];

        // Call the function to get the wakeup order
        let result = determine_wakeup_order(&servers).expect("Failed to determine wakeup order");

        // Check that the result matches the expected order
        assert_eq!(
            result.into_iter().map(|s| s.name).collect::<Vec<String>>(),
            expected_order
        );
    }
}
