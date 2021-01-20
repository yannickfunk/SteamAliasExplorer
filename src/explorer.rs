use crate::explorer::PriorityMessage::{FirstLevel, SecondLevel};
use mysql::prelude::Queryable;
use mysql::{Params, Pool};
use reqwest::{Client, Error};
use serde_json::{Map, Value};
use std::str::FromStr;
use std::time::SystemTime;
use tokio::sync::broadcast::{self, error::TryRecvError, Receiver, Sender};
use tokio::time::Duration;

#[derive(Clone, Debug)]
pub enum PriorityMessage {
    FirstLevel(u64),
    SecondLevel(u64),
}

pub struct Explorer {
    rx_external: Receiver<u64>,
    tx_internal: Sender<u64>,
    tx_priority: Sender<PriorityMessage>,
    conn_pool: Pool,
    client: Client,
}

impl Explorer {
    pub fn new(rx_external: Receiver<u64>, conn_pool: Pool, client: Client) -> Self {
        let (tx_internal, _) = broadcast::channel(1_000_000);
        let (tx_priority, _) = broadcast::channel(100_000);
        Self {
            rx_external,
            tx_internal,
            tx_priority,
            conn_pool,
            client,
        }
    }

    pub async fn start(&mut self) {
        let mut rx_internal = self.tx_internal.subscribe();
        let mut rx_priority = self.tx_priority.subscribe();
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            match self.rx_external.try_recv() {
                Ok(id_64) => {
                    if id_64 == 0 {
                        break;
                    } else {
                        tokio::spawn(explore_priority(
                            self.client.clone(),
                            self.tx_priority.clone(),
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
            if let Ok(priority_message) = rx_priority.try_recv() {
                match priority_message {
                    FirstLevel(user) => tokio::spawn(try_retrieve_and_write_aliases(
                        self.conn_pool.clone(),
                        self.client.clone(),
                        self.tx_internal.clone(),
                        user,
                    )),
                    SecondLevel(user) => tokio::spawn(explore(
                        self.conn_pool.clone(),
                        self.client.clone(),
                        self.tx_internal.clone(),
                        user,
                    )),
                };
            } else if let Ok(next_user) = rx_internal.try_recv() {
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
    try_retrieve_and_write_aliases(conn_pool, client.clone(), tx_internal.clone(), next_user).await;
    put_friends_on_queue(client, tx_internal, next_user).await;
}

async fn explore_priority(client: Client, tx_priority: Sender<PriorityMessage>, next_user: u64) {
    if let Err(e) = tx_priority.send(FirstLevel(next_user)) {
        log::error!("Error putting user on the queue: {:?}", e)
    }

    match retrieve_friends(client.clone(), next_user).await {
        Ok(friends) => {
            for friend in friends {
                if let Err(e) = tx_priority.send(FirstLevel(friend)) {
                    log::error!("Error putting user on the queue: {:?}", e)
                }
                match retrieve_friends(client.clone(), friend).await {
                    Ok(friends) => friends.iter().for_each(|e| {
                        if let Err(e) = tx_priority.send(SecondLevel(*e)) {
                            log::error!("Error putting user on the queue: {:?}", e)
                        }
                    }),
                    Err(e) => log::error!("Error retrieving friends: {:?}", e),
                };
            }
        }
        Err(e) => log::error!("Error retrieving friends of priority user: {:?}", e),
    };
}

async fn try_retrieve_and_write_aliases(
    conn_pool: Pool,
    client: Client,
    tx_internal: Sender<u64>,
    user: u64,
) {
    match get_aliases(client.clone(), user).await {
        Ok(aliases) => {
            let aliases = if aliases.len() == 0 {
                if let Ok(current_name) = get_current_name(client.clone(), user).await {
                    vec![current_name]
                } else {
                    log::error!("Could not get current name for: {:?}", user);
                    return;
                }
            } else {
                aliases
            };
            if let Err(e) = write_aliases_db(conn_pool, user, aliases).await {
                log::error!("Error writing aliases into database: {:?}", e);
            }
        }
        Err(e) => {
            log::error!("Error retrieving aliases: {:?}, back on the Queue", e);
            if let Err(e) = tx_internal.send(user) {
                log::error!("Error putting user on the queue: {:?}", e)
            }
        }
    };
}

async fn put_friends_on_queue(client: Client, tx: Sender<u64>, user: u64) {
    match retrieve_friends(client, user).await {
        Ok(friends) => friends.iter().for_each(|e| {
            if let Err(e) = tx.send(*e) {
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
        std::env::var("api_webkey").unwrap(),
        id_64
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
    mut aliases: Vec<String>,
) -> Result<(), mysql::Error> {
    let query = format!("SELECT id_64 from aliases WHERE id_64 = {:?}", id_64);
    let query_result: Vec<String> = conn_pool.get_conn()?.query(query)?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    if query_result.len() > 0 {
        let aliases_from_db = get_aliases_from_db(conn_pool.clone(), id_64).await?;
        let new_aliases: Vec<String> = aliases
            .iter()
            .filter(|e| !aliases_from_db.contains(e))
            .map(|e| e.to_string())
            .collect();
        log::info!("New aliases: {:?}", new_aliases.clone());
        aliases.extend(new_aliases);
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
    let request = format!("http://api.steampowered.com/ISteamUser/GetFriendList/v0001/?key={:}&steamid={:?}&relationship=friend", std::env::var("api_webkey").unwrap(), id_64);
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

async fn get_aliases_from_db(conn_pool: Pool, user: u64) -> Result<Vec<String>, mysql::Error> {
    let mut db_conn = conn_pool.get_conn()?;
    let query = format!("SELECT alias_list from aliases WHERE id_64 = {:?}", user);
    let query_result: Vec<String> = db_conn.query(query)?;
    let json_str = query_result.first().unwrap().as_str();
    let aliases: Vec<String> = json_str
        .strip_prefix("[")
        .unwrap()
        .strip_suffix("]")
        .unwrap()
        .split(", ")
        .map(|e| e.replace("\"", "").to_string())
        .collect();
    Ok(aliases)
}
