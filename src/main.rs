mod explorer;

use crate::explorer::Explorer;
use mysql::Pool;
use reqwest::{self, Client};
use std::time::SystemTime;
use tide::StatusCode;
use tokio::sync::broadcast::{self, Sender};

#[tokio::main]
async fn main() {
    setup_logger().unwrap();

    let mysql_password = std::env::var("mysql_password").unwrap();
    let mysql_hostname = std::env::var("mysql_hostname").unwrap();
    let mysql_username = std::env::var("mysql_username").unwrap();
    let mysql_database = std::env::var("mysql_database").unwrap();
    let url = format!(
        "mysql://{:}:{:}@{:}/{:}",
        mysql_username, mysql_password, mysql_hostname, mysql_database
    );
    let pool = Pool::new(url).unwrap();
    let client = Client::builder().connection_verbose(false).build().unwrap();

    let (tx, rx) = broadcast::channel(10000);
    let mut explorer = Explorer::new(rx, pool, client);
    let explorer_future = async move {
        explorer.start().await;
    };

    tokio::spawn(explorer_future);
    run_webserver(tx.clone()).await;

    loop {}
}

async fn run_webserver(tx: Sender<u64>) {
    let mut app = tide::new();
    app.at("/explorer/enqueue/:id_64")
        .get(move |req: tide::Request<()>| {
            let tx = tx.clone();
            async move {
                let id_64 = req.param("id_64")?.parse()?;
                if let Err(_) = tx.send(id_64) {
                    Err(tide::Error::from_str(
                        StatusCode::BadGateway,
                        "Queue not reachable",
                    ))
                } else {
                    Ok(format!("Enqueued {:?}", id_64))
                }
            }
        });
    app.listen("0.0.0.0:8080").await.unwrap();
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
                    "{:?}.log",
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
