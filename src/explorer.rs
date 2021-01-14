use crate::STEAM_API_KEY;
use mysql::prelude::Queryable;
use mysql::{Params, Pool};
use reqwest::{Client, Error};
use serde_json::{from_str, Map, Value};
use std::collections::VecDeque;
use std::str::FromStr;
use std::time::SystemTime;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::broadcast::Receiver;

pub struct Explorer {
    rx: Receiver<u64>,
    explore_queue: VecDeque<u64>,
    conn_pool: Pool,
    client: Client,
}

impl Explorer {
    pub fn new(rx: Receiver<u64>, conn_pool: Pool, client: Client) -> Self {
        Self {
            rx,
            explore_queue: VecDeque::new(),
            conn_pool,
            client,
        }
    }

    pub async fn start(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(id_64) => {
                    if id_64 == 0 {
                        break;
                    } else {
                        self.explore_queue.push_front(id_64);
                    }
                }
                Err(e) => match e {
                    TryRecvError::Empty => {}
                    TryRecvError::Closed => break,
                    TryRecvError::Lagged(_) => {
                        println!("lagged!")
                    }
                },
            };
            if let Some(next_user) = self.explore_queue.pop_front() {
                self.explore(next_user).await;
            }
        }
    }

    async fn explore(&mut self, next_user: u64) {
        println!("Exploring {:?}", next_user);
        match self.get_aliases(next_user).await {
            Ok(aliases) => {
                let aliases = if aliases.len() == 0 {
                    if let Ok(current_name) = self.get_current_name(next_user).await {
                        vec![current_name]
                    } else {
                        println!("Could not get current name!");
                        return;
                    }
                } else {
                    aliases
                };
                if let Err(e) = self.write_aliases_db(next_user, aliases).await {
                    println!("Error writing aliases into database: {:?}", e);
                }
            }
            Err(e) => {
                println!(
                    "Error retrieving aliases: {:?}, pushing back on the Queue",
                    e
                );
                self.explore_queue.push_back(next_user)
            }
        };
        println!("{:?}", self.explore_queue.len());
        if self.explore_queue.len() < 1_000_000 {
            match self.retrieve_friends(next_user).await {
                Ok(friends) => friends
                    .iter()
                    .for_each(|e| self.explore_queue.push_back(*e)),
                Err(e) => println!("Error retrieving friends: {:?}", e),
            };
        }
    }

    async fn retrieve_friends(&mut self, id_64: u64) -> Result<Vec<u64>, Error> {
        let request = format!("http://api.steampowered.com/ISteamUser/GetFriendList/v0001/?key={:}&steamid={:?}&relationship=friend", STEAM_API_KEY, id_64);
        let text = self
            .client
            .get(&request)
            .send()
            .await?
            .json::<Value>()
            .await?;
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
        println!("No public friendlist or Error retrieving friends");
        Ok(vec![])
    }

    async fn get_aliases(&mut self, id_64: u64) -> Result<Vec<String>, Error> {
        let request = format!("http://steamcommunity.com/profiles/{:}/ajaxaliases/", id_64);
        let text = self
            .client
            .get(&request)
            .send()
            .await?
            .json::<Value>()
            .await?;
        let ser_vec: Vec<Value> = text.as_array().unwrap().clone();
        Ok(ser_vec
            .iter()
            .map(|e| e["newname"].to_string().replace("\"", ""))
            .collect())
    }

    async fn write_aliases_db(
        &mut self,
        id_64: u64,
        aliases: Vec<String>,
    ) -> Result<(), mysql::Error> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let stmt = format!(
            "INSERT INTO aliases (id_64, alias_list, last_written) VALUES ({:?}, '{:?}', {:?})",
            id_64, aliases, now
        );
        let _: Vec<String> = self.conn_pool.get_conn()?.exec(stmt, Params::Empty)?;
        Ok(())
    }

    async fn get_current_name(&mut self, id_64: u64) -> Result<String, Error> {
        let request = format!(
            "http://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/?key={:}&steamids={:?}",
            STEAM_API_KEY, id_64
        );
        let text = self
            .client
            .get(&request)
            .send()
            .await?
            .json::<Value>()
            .await?;
        let json: Map<String, Value> = text.as_object().unwrap().clone();
        Ok(json["response"]["players"][0]["personaname"]
            .to_string()
            .replace("\"", ""))
    }
}
