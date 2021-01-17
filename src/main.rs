mod explorer;
use crate::explorer::Explorer;
use log::{error, info, trace, warn};
use mysql::Pool;
use reqwest::{self, Client};
use std::time::SystemTime;
use tokio::sync::broadcast;
use tokio::time::{sleep, Duration};

const STEAM_API_KEY: &str = "E564DC18D4C81A716362FC53104572D5";

#[tokio::main]
async fn main() {
    setup_logger().unwrap();

    let url = "mysql://root:test@localhost:3306/alias_explorer";
    let pool = Pool::new(url).unwrap();
    let client = Client::builder().connection_verbose(false).build().unwrap();

    let (tx, rx) = broadcast::channel(10000);
    let mut explorer = Explorer::new(rx, pool, client);
    let explorer_future = async move {
        explorer.start().await;
    };

    tokio::spawn(explorer_future);
    let id_to_send_to_service = 76561198248077092;
    tx.send(id_to_send_to_service).unwrap();
    sleep(Duration::from_secs(60 * 60 * 24)).await;
}

fn setup_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .chain(
            fern::Dispatch::new()
                .level(log::LevelFilter::Debug)
                .chain(fern::log_file(format!(
                    "log/{:?}.log",
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64()
                ))?),
        )
        .chain(
            fern::Dispatch::new()
                .level(log::LevelFilter::Info)
                .chain(std::io::stdout()),
        )
        .apply()?;

    Ok(())
}
