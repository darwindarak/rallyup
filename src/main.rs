use std::io::{stdout, Write};
use std::{
    env,
    sync::{Arc, Mutex},
};
mod servers;
mod wol;
use colored::*;
use crossterm::{
    execute,
    style::Print,
    terminal::{Clear, ClearType},
};
use tokio::time::sleep;

#[derive(Debug, Clone)]
enum ServerStatus {
    Waiting,
    WOLSent,
    Ok,
}

#[derive(Debug, Clone)]
enum CheckStatus {
    Waiting(String),
    Running(String),
    Ok(String),
}

// const SPINNER: &[&str] = &["|", "/", "-", "\\"];
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone)]
struct Server {
    name: String,
    status: ServerStatus,
    checks: Vec<CheckStatus>,
}

fn render_servers(servers: &Vec<Server>, spinner_index: usize) {
    let mut stdout = stdout();

    // Clear the screen
    execute!(stdout, Clear(ClearType::All)).unwrap();

    for server in servers {
        // Display the server name and status
        let (icon, server_status) = match server.status {
            ServerStatus::Waiting => ("◉".normal(), "waiting".normal()),
            ServerStatus::WOLSent => ("◉".yellow(), "WOL sent".yellow()),
            ServerStatus::Ok => ("◉".green(), "ok".green()),
        };
        execute!(
            stdout,
            Print(format!(
                "{} {} [{}]\n",
                icon,
                server.name.bold(),
                server_status
            ))
        )
        .unwrap();

        for (i, check) in server.checks.iter().enumerate() {
            let mut extension = "│";
            if i == server.checks.len() - 1 {
                execute!(stdout, Print("└──")).unwrap();
                extension = " ";
            } else {
                execute!(stdout, Print("├──")).unwrap();
            }
            match check {
                CheckStatus::Waiting(name) => {
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}    └── Status: {}\n",
                            name,
                            extension,
                            "waiting".yellow()
                        ))
                    )
                    .unwrap();
                }
                CheckStatus::Running(name) => {
                    let spinner = SPINNER[spinner_index % SPINNER.len()];
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}   └── Status: {}\n",
                            name, extension, spinner
                        ))
                    )
                    .unwrap();
                }
                CheckStatus::Ok(name) => {
                    execute!(
                        stdout,
                        Print(format!(
                            "{}\n{}   └── Status: {}\n",
                            name,
                            extension,
                            "ok".green()
                        ))
                    )
                    .unwrap();
                }
            }
        }
        execute!(stdout, Print("\n")).unwrap();
    }
    stdout.flush().unwrap();
}

async fn update_server_status(servers: Arc<Mutex<Vec<Server>>>) {
    let mut spinner_index = 0;

    loop {
        {
            let servers = servers.lock().unwrap();
            render_servers(&servers, spinner_index);
        }

        spinner_index = (spinner_index + 1) % SPINNER.len();

        sleep(std::time::Duration::from_millis(200)).await;
    }
}

async fn perform_health_checks(
    server: &servers::Server,
    server_state: Arc<Mutex<Vec<Server>>>,
    server_index: usize,
) {
    let mut tasks = Vec::new();
    let name = server.name.clone();
    let checks = server.check.clone();

    for (check_index, check) in checks.into_iter().enumerate() {
        // TODO: Can implement display later, using debug for now
        let check_display = format!("{:?}", check);
        {
            let mut servers = server_state.lock().unwrap();
            servers[server_index].checks[check_index] = CheckStatus::Running(check_display.clone());
        }

        let check = check.clone();
        let name = name.clone();
        let server_state = Arc::clone(&server_state);

        tasks.push(tokio::spawn(async move {
            loop {
                let result = servers::check_health(check.clone()).await;
                if result {
                    break;
                } else {
                    servers::check_wait(check.clone()).await
                }
            }
            {
                let mut servers = server_state.lock().unwrap();
                servers[server_index].checks[check_index] = CheckStatus::Ok(check_display.clone());
            }
        }))
    }
    for task in tasks {
        task.await.unwrap();
    }
    {
        let mut servers = server_state.lock().unwrap();
        servers[server_index].status = ServerStatus::Ok;
    }
}

fn print_help() {
    println!("Usage: wakeup <file>");
    println!("wakeup: A tool to send Wake-on-LAN packets to servers in dependency order");
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        print_help();
        return Ok(());
    }

    let filename = &args[1];

    let wake_order = servers::parse_server_dependencies(filename)?;
    let server_state = Arc::new(Mutex::new(
        wake_order
            .iter()
            .map(|server| Server {
                name: server.name.clone(),
                status: ServerStatus::Waiting,
                checks: server
                    .check
                    .iter()
                    .map(|check| CheckStatus::Waiting(format!("{:?}", check)))
                    .collect(),
            })
            .collect(),
    ));

    let server_state_ptr = Arc::clone(&server_state);
    tokio::spawn(async move {
        update_server_status(server_state_ptr).await;
    });

    for (server_index, server) in wake_order.into_iter().enumerate() {
        wol::send_wol_packet(&server.mac, &server.interface, server.vlan)?;
        {
            let mut servers = server_state.lock().unwrap();
            servers[server_index].status = ServerStatus::WOLSent;
        }

        perform_health_checks(&server, server_state.clone(), server_index).await
    }

    render_servers(&server_state.lock().unwrap(), 0);
    return Ok(());
}
