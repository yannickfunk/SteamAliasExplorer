use crate::STEAM_API_KEY;
use futures::future::join_all;
use reqwest::{Client, Error};
use serde::Serialize;
use serde_json::{from_str, Map, Value};
use std::str::FromStr;
use mysql::{Pool, Params::Empty};
use mysql::prelude::Queryable;

#[derive(Debug, Serialize)]
pub struct SteamUser {
    id_64: u64,
    aliases: Vec<String>,
    pub friends: Option<Vec<SteamUser>>,
    #[serde(skip_serializing)]
    client: Client,
    #[serde(skip_serializing)]
    conn_pool: Pool
}

impl SteamUser {
    pub async fn new_from_id_64(conn_pool: Pool, client: &Client, id_64: u64) -> Self {
        let retrieved_aliases = get_aliases(client, id_64).await.unwrap();
        let aliases = if retrieved_aliases.len() > 0 {
            retrieved_aliases
        } else {
            vec![get_current_name(client, id_64).await.unwrap()]
        };

        let stmt = format!("INSERT INTO aliases (id_64, alias_list) VALUES ({:?}, '{:?}')", id_64, aliases);
        let _: Vec<String> = conn_pool.get_conn().unwrap().exec(stmt, Empty).unwrap();

        Self {
            id_64,
            aliases,
            friends: None,
            client: client.clone(),
            conn_pool
        }
    }

    pub async fn retrieve_friends(&mut self) -> Result<(), Error> {
        let request = format!("http://api.steampowered.com/ISteamUser/GetFriendList/v0001/?key={:}&steamid={:?}&relationship=friend", STEAM_API_KEY, self.id_64);
        let text = self.client.get(&request).send().await?.text().await?;
        let parsed: Value = from_str(&text).unwrap();
        let json: Map<String, Value> = parsed.as_object().unwrap().clone();
        let friend_ids: Vec<u64> = json["friendslist"]["friends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| u64::from_str(&e["steamid"].to_string().replace("\"", "")).unwrap())
            .collect();
        let friends_join = friend_ids
            .iter()
            .map(|id| Self::new_from_id_64(self.conn_pool.clone(), &self.client, *id));
        let friends = join_all(friends_join).await;
        self.friends = Some(friends);
        Ok(())
    }
}

async fn get_aliases(client: &Client, id_64: u64) -> Result<Vec<String>, Error> {
    let request = format!("http://steamcommunity.com/profiles/{:}/ajaxaliases/", id_64);
    let send_result = client.get(&request).send().await;
    if let Err(_) = send_result {
        return Ok(vec!["failed".to_owned()]);
    }
    let text = send_result?.text().await?;
    let parsed_res = from_str(&text);
    if let Err(_) = parsed_res {
        return Ok(vec!["failed".to_owned()]);
    }
    let parsed: Value = parsed_res.unwrap();
    let ser_vec: Vec<Value> = parsed.as_array().unwrap().clone();
    Ok(ser_vec
        .iter()
        .map(|e| e["newname"].to_string().replace("\"", ""))
        .collect())
}

async fn get_current_name(client: &Client, id_64: u64) -> Result<String, Error> {
    let request = format!(
        "http://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/?key={:}&steamids={:?}",
        STEAM_API_KEY, id_64
    );
    let text = client.get(&request).send().await?.text().await?;
    let parsed: Value = from_str(&text).unwrap();
    let json: Map<String, Value> = parsed.as_object().unwrap().clone();
    Ok(json["response"]["players"][0]["personaname"]
        .to_string()
        .replace("\"", ""))
}
