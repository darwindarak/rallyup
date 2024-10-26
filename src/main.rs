mod servers;
mod wol;

use colored::*;
use crossterm::{
    execute,
    style::Print,
    terminal::{Clear, ClearType},
};
use std::io::{stdout, Write};
use std::{env, sync::Arc};
use tokio::{sync::RwLock, time::sleep};

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_servers(servers: &Vec<servers::Server>, spinner_index: usize, backtrack: u16) -> u16 {
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
            servers::ServerStatus::Waiting => ("◉".normal(), "waiting".normal()),
            servers::ServerStatus::WOLSent => ("◉".yellow(), "WOL sent".yellow()),
            servers::ServerStatus::Ok => ("◉".green(), "ok".green()),
            servers::ServerStatus::TimedOut => ("◉".red(), "timed-out".red()),
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

        for (i, check) in server.check.iter().enumerate() {
            let mut extension = "│";
            if i == server.check.len() - 1 {
                execute!(stdout, Print("└──")).unwrap();
                extension = " ";
            } else {
                execute!(stdout, Print("├──")).unwrap();
            }
            match check.status {
                servers::CheckStatus::Waiting => {
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}    └── Status: {}\n",
                            check,
                            extension,
                            "waiting".yellow()
                        ))
                    )
                    .unwrap();
                }
                servers::CheckStatus::TimedOut => {
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}    └── Status: {}\n",
                            check,
                            extension,
                            "timed-out".red()
                        ))
                    )
                    .unwrap();
                }

                servers::CheckStatus::Running => {
                    let spinner = SPINNER[spinner_index % SPINNER.len()];
                    execute!(
                        stdout,
                        Print(format!(
                            " {}\n{}   └── Status: {}\n",
                            check, extension, spinner
                        ))
                    )
                    .unwrap();
                }
                servers::CheckStatus::Ok => {
                    execute!(
                        stdout,
                        Print(format!(
                            "{}\n{}   └── Status: {}\n",
                            check,
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

async fn update_server_status(servers: Arc<RwLock<Vec<servers::Server>>>) {
    let mut spinner_index = 0;
    let mut last_line_count = 0;

    loop {
        {
            let servers = servers.read().await;
            last_line_count = render_servers(&servers, spinner_index, last_line_count);
        }

        spinner_index = (spinner_index + 1) % SPINNER.len();

        sleep(std::time::Duration::from_millis(200)).await;
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

    let mut line_count = 0;
    for server in wake_order.iter() {
        // server status line
        line_count += 1;
        // 2 lines per health check
        line_count += server.check.len() as u16 * 2;
        // newline between servers
        line_count += 1;
    }

    // Need to keep it in a Arc<RwLock> since the status render loop will be reading
    // the server status while the health checks may be updating it concurrently
    let servers = Arc::new(RwLock::new(wake_order.clone()));

    tokio::spawn(update_server_status(servers.clone()));

    for (server_index, server) in wake_order.into_iter().enumerate() {
        wol::send_wol_packet(&server.mac, &server.interface, server.vlan)?;
        {
            let mut servers = servers.write().await;
            servers[server_index].status = servers::ServerStatus::WOLSent;
        }

        let server_status = servers::perform_health_checks(servers.clone(), server_index).await;

        if let servers::ServerStatus::TimedOut = server_status {
            let servers = servers.read().await;
            render_servers(&servers, 0, line_count);
            return Err(anyhow::anyhow!(
                "health check for {} timed out",
                server.name
            ));
        }
    }

    {
        let servers = servers.read().await;
        render_servers(&servers, 0, line_count);
    }
    return Ok(());
}
