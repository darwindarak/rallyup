mod servers;
mod wol;

use colored::*;
use crossterm::{
    execute,
    style::Print,
    terminal::{Clear, ClearType},
};
use std::io::{stdout, Write};
use std::{
    env,
    sync::{Arc, Mutex},
};
use tokio::time::{sleep, Instant};

#[derive(Debug, Clone)]
enum ServerStatus {
    Waiting,
    WOLSent,
    Ok,
    TimedOut,
}

#[derive(Debug, Clone)]
enum CheckStatus {
    Waiting(String),
    Running(String),
    TimedOut(String),
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

fn render_servers(servers: &Vec<Server>, spinner_index: usize, backtrack: u16) -> u16 {
    let mut stdout = stdout();

    // Clear what was previously rendered
    if backtrack > 0 {
        execute!(
            stdout,
            crossterm::cursor::MoveToPreviousLine(backtrack),
            Clear(ClearType::FromCursorDown)
        )
        .unwrap();
    }

    let mut line_count = 0;

    for server in servers {
        // Display the server name and status
        let (icon, server_status) = match server.status {
            ServerStatus::Waiting => ("◉".normal(), "waiting".normal()),
            ServerStatus::WOLSent => ("◉".yellow(), "WOL sent".yellow()),
            ServerStatus::Ok => ("◉".green(), "ok".green()),
            ServerStatus::TimedOut => ("◉".red(), "timed-out".red()),
        };
        execute!(
            stdout,
            Print(format!(
                "{} {}: {}\n",
                icon,
                server.name.bold(),
                server_status
            ))
        )
        .unwrap();
        line_count += 1;

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
                CheckStatus::TimedOut(name) => {
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}    └── Status: {}\n",
                            name,
                            extension,
                            "timed-out".red()
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
            line_count += 2;
        }
        execute!(stdout, Print("\n")).unwrap();
        line_count += 1;
    }
    stdout.flush().unwrap();
    line_count
}

async fn update_server_status(servers: Arc<Mutex<Vec<Server>>>) {
    let mut spinner_index = 0;
    let mut last_line_count = 0;

    loop {
        {
            let servers = servers.lock().unwrap();
            last_line_count = render_servers(&servers, spinner_index, last_line_count as u16);
        }

        spinner_index = (spinner_index + 1) % SPINNER.len();

        sleep(std::time::Duration::from_millis(200)).await;
    }
}

async fn perform_health_checks(
    server: &servers::Server,
    server_state: Arc<Mutex<Vec<Server>>>,
    server_index: usize,
) -> ServerStatus {
    let mut tasks = Vec::new();
    let checks = server.check.clone();

    for (check_index, check) in checks.into_iter().enumerate() {
        let check_display = format!("{}", check);
        {
            let mut servers = server_state.lock().unwrap();
            servers[server_index].checks[check_index] = CheckStatus::Running(check_display.clone());
        }

        let check = check.clone();
        let server_state = Arc::clone(&server_state);

        tasks.push(tokio::spawn(async move {
            let start_time = Instant::now();
            loop {
                if start_time.elapsed() >= check.timeout {
                    {
                        let mut servers = server_state.lock().unwrap();
                        servers[server_index].checks[check_index] =
                            CheckStatus::TimedOut(check_display.clone());
                    }
                    return CheckStatus::TimedOut(check_display.clone());
                }
                let result = servers::check_health(check.method.clone()).await;
                if result {
                    break;
                } else {
                    tokio::time::sleep(check.retry).await;
                }
            }
            {
                let mut servers = server_state.lock().unwrap();
                servers[server_index].checks[check_index] = CheckStatus::Ok(check_display.clone());
            }
            return CheckStatus::Ok(check_display.clone());
        }))
    }
    let mut timeout = false;
    for task in tasks {
        if let CheckStatus::TimedOut(_) = task.await.unwrap() {
            timeout = true;
        }
    }
    {
        let mut servers = server_state.lock().unwrap();
        servers[server_index].status = if timeout {
            ServerStatus::TimedOut
        } else {
            ServerStatus::Ok
        };
    }

    if timeout {
        ServerStatus::TimedOut
    } else {
        ServerStatus::Ok
    }
}

fn print_help() {
    println!("Usage: spinup <file>");
    println!("spinup: A tool to send Wake-on-LAN packets to servers in dependency order");
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
                    .map(|check| CheckStatus::Waiting(format!("{}", check)))
                    .collect(),
            })
            .collect(),
    ));
    let mut line_count = 0;
    for server in wake_order.iter() {
        // server status line
        line_count += 1;
        // 2 lines per health check
        line_count += server.check.len() as u16 * 2;
        // newline between servers
        line_count += 1;
    }

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

        if let ServerStatus::TimedOut =
            perform_health_checks(&server, server_state.clone(), server_index).await
        {
            render_servers(&server_state.lock().unwrap(), 0, line_count);
            return Err(anyhow::anyhow!(
                "health check for {} timed out",
                server.name
            ));
        }
    }

    render_servers(&server_state.lock().unwrap(), 0, line_count);
    return Ok(());
}
