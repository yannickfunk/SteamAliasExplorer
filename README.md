# SteamAliasExplorer
Service which caches all related steam profiles and their recent aliases starting from an enqueued user


## Requirements
 * MySQL Database
     * Table `aliases`
         ```SQL
         CREATE TABLE `aliases` (
	           `id_64` BIGINT(20) UNSIGNED NOT NULL,
	           `alias_list` LONGTEXT NULL DEFAULT NULL COLLATE 'utf8mb4_bin',
	           `last_written` DOUBLE(22,0) NULL DEFAULT NULL,
	           PRIMARY KEY (`id_64`) USING BTREE
         )
         ```
 * Environment Variables
      * mysql_database
      * mysql_hostname
      * mysql_password
      * mysql_username
      * api_webkey (This is your Steam API key)

## Build and Run
```
cargo run --release
```

## Put users on the queue to explore:

```
http://127.0.0.1:8080/explorer/enqueue/<steamid_64>
```

The service now collects the users friends, the friend's friends and so on..
It writes all users it comes across into the database
