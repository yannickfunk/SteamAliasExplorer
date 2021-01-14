mod explorer;

use crate::explorer::Explorer;
use mysql::Pool;
use reqwest::{self, Client};
use std::env;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tokio::time::Duration;

const STEAM_API_KEY: &str = "E564DC18D4C81A716362FC53104572D5";

#[tokio::main]
async fn main() {
    let url = "mysql://root:test@localhost:3306/alias_explorer";
    let pool = Pool::new(url).unwrap();
    let client = Client::new();

    let (tx, rx) = broadcast::channel(10000);
    let mut explorer = Explorer::new(rx, pool, client);
    let explorer_future = async move {
        explorer.start().await;
    };

    let id_to_send_to_service = 76561198021323440;
    tokio::spawn(explorer_future);
    tx.send(id_to_send_to_service).unwrap();
    sleep(Duration::from_secs(60 * 60 * 12)).await;
}
