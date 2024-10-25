mod servers;
mod wol;

async fn perform_health_checks(server: &servers::Server) {
    let mut tasks = Vec::new();
    let name = server.name.clone();

    for check in &server.check {
        let check = check.clone();
        let name = name.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                let result = servers::check_health(check.clone()).await;
                if result {
                    break;
                } else {
                    println!("server {} not ready, waiting...", name);
                    servers::check_wait(check.clone()).await
                }
            }
        }))
    }
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let wake_order = servers::parse_server_dependencies("./sample.yml")?;
    for server in wake_order {
        wol::send_wol_packet(&server.mac, &server.interface, server.vlan)?;
        println!("Sent WOL to {}", server.name);

        perform_health_checks(&server).await
    }
    return Ok(());
}
