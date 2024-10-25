mod servers;
//mod wol;

fn main() -> Result<(), anyhow::Error> {
    let servers = servers::parse_server_dependencies("./sample.yml")?;
    println!("{:?}", servers);

    return Ok(());
}
