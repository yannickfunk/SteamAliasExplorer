use crate::STEAM_API_KEY;
use mysql::prelude::Queryable;
use mysql::{Params, Pool};
use reqwest::{Client, Error};
use serde_json::{Map, Value};
use std::str::FromStr;
use std::time::SystemTime;
use tokio::sync::broadcast::{self, error::TryRecvError, Receiver, Sender};
use tokio::time::Duration;

pub struct Explorer {
    rx_external: Receiver<u64>,
    rx_internal: Receiver<u64>,
    tx_internal: Sender<u64>,
    conn_pool: Pool,
    client: Client,
}

impl Explorer {
    pub fn new(rx_external: Receiver<u64>, conn_pool: Pool, client: Client) -> Self {
        let (tx_internal, rx_internal) = broadcast::channel(1_000_000);
        Self {
            rx_external,
            rx_internal,
            tx_internal,
            conn_pool,
            client,
        }
    }

    pub async fn start(&mut self) {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            match self.rx_external.try_recv() {
                Ok(id_64) => {
                    if id_64 == 0 {
                        break;
                    } else {
                        tokio::spawn(explore(
                            self.conn_pool.clone(),
                            self.client.clone(),
                            self.tx_internal.clone(),
                            id_64,
                        ));
                    }
                }
                Err(e) => match e {
                    TryRecvError::Empty => {}
                    TryRecvError::Closed => break,
                    TryRecvError::Lagged(e) => {
                        log::error!("{:?}", e);
                    }
                },
            };
            if let Ok(next_user) = self.rx_internal.try_recv() {
                tokio::spawn(explore(
                    self.conn_pool.clone(),
                    self.client.clone(),
                    self.tx_internal.clone(),
                    next_user,
                ));
            }
        }
    }
}

async fn explore(conn_pool: Pool, client: Client, tx_internal: Sender<u64>, next_user: u64) {
    match get_aliases(client.clone(), next_user).await {
        Ok(aliases) => {
            let aliases = if aliases.len() == 0 {
                if let Ok(current_name) = get_current_name(client.clone(), next_user).await {
                    vec![current_name]
                } else {
                    log::error!("Could not get current name for: {:?}", next_user);
                    return;
                }
            } else {
                aliases
            };
            if let Err(e) = write_aliases_db(conn_pool, next_user, aliases).await {
                log::error!("Error writing aliases into database: {:?}", e);
            }
        }
        Err(e) => {
            log::error!("Error retrieving aliases: {:?}, back on the Queue", e);
            if let Err(e) = tx_internal.send(next_user) {
                log::error!("Error putting user on the queue: {:?}", e)
            }
        }
    };
    match retrieve_friends(client, next_user).await {
        Ok(friends) => friends.iter().for_each(|e| {
            if let Err(e) = tx_internal.send(*e) {
                log::error!("Error putting user on the queue: {:?}", e)
            }
        }),
        Err(e) => log::error!("Error retrieving friends: {:?}", e),
    };
}

async fn get_aliases(client: Client, id_64: u64) -> Result<Vec<String>, Error> {
    let request = format!("http://steamcommunity.com/profiles/{:}/ajaxaliases/", id_64);
    let text = client.get(&request).send().await?.json::<Value>().await?;
    let ser_vec: Vec<Value> = text.as_array().unwrap().clone();
    Ok(ser_vec
        .iter()
        .map(|e| e["newname"].to_string().replace("\"", ""))
        .collect())
}

async fn get_current_name(client: Client, id_64: u64) -> Result<String, Error> {
    let request = format!(
        "http://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/?key={:}&steamids={:?}",
        STEAM_API_KEY, id_64
    );
    let text = client.get(&request).send().await?.json::<Value>().await?;
    let json: Map<String, Value> = text.as_object().unwrap().clone();
    Ok(json["response"]["players"][0]["personaname"]
        .to_string()
        .replace("\"", ""))
}

async fn write_aliases_db(
    conn_pool: Pool,
    id_64: u64,
    aliases: Vec<String>,
) -> Result<(), mysql::Error> {
    let query = format!("SELECT id_64 from aliases WHERE id_64 = {:?}", id_64);
    let query_result: Vec<String> = conn_pool.get_conn()?.query(query)?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    if query_result.len() > 0 {
        let update_stmt = format!(
            "UPDATE aliases SET alias_list = '{:?}', last_written = {:?} WHERE id_64 = {:?}",
            aliases, now, id_64
        );
        let _: Vec<String> = conn_pool
            .get_conn()?
            .exec(update_stmt.clone(), Params::Empty)?;
        log::info!("Updated:  {:}", update_stmt);
    } else {
        let insert_stmt = format!(
            "INSERT INTO aliases (id_64, alias_list, last_written) VALUES ({:?}, '{:?}', {:?})",
            id_64, aliases, now
        );
        let _: Vec<String> = conn_pool
            .get_conn()?
            .exec(insert_stmt.clone(), Params::Empty)?;
        log::info!("Inserted:  {:}", insert_stmt);
    }
    Ok(())
}

async fn retrieve_friends(client: Client, id_64: u64) -> Result<Vec<u64>, Error> {
    let request = format!("http://api.steampowered.com/ISteamUser/GetFriendList/v0001/?key={:}&steamid={:?}&relationship=friend", STEAM_API_KEY, id_64);
    let text = client.get(&request).send().await?.json::<Value>().await?;
    let json: Map<String, Value> = text.as_object().unwrap().clone();
    if let Some(friendlist) = json.get("friendslist") {
        if let Some(friends) = friendlist.get("friends") {
            let friend_ids: Vec<u64> = friends
                .as_array()
                .unwrap()
                .iter()
                .map(|e| u64::from_str(&e["steamid"].to_string().replace("\"", "")).unwrap())
                .collect();
            return Ok(friend_ids);
        }
    }
    log::error!("No public friendlist or Error retrieving friends");
    Ok(vec![])
}
