use crate::github_api::GitHubApi;
use futures::lock::Mutex;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use showdown::url::Url;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fmt::{self, Formatter};
use std::hash::{BuildHasher, Hash, Hasher};
use std::slice;
use unicase::UniCase;

pub struct Config {
    pub server: Url,
    pub user: String,
    pub password: String,
    pub secret: String,
    pub port: u16,
    default_room_name: Option<String>,
    room_configuration: HashMap<String, RoomConfiguration>,
    pub github_api: Option<Mutex<GitHubApi>>,
    pub username_aliases: UsernameAliases,
}

#[derive(Default)]
pub struct UsernameAliases {
    map: hashbrown::HashMap<UniCase<String>, String>,
}

impl UsernameAliases {
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        let unicase = UniCase::new(key);
        let mut hasher = self.map.hasher().build_hasher();
        unicase.hash(&mut hasher);
        self.map
            .raw_entry()
            .from_hash(hasher.finish(), |k| *k == unicase)
            .map_or(key, |(_, v)| v)
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.map.insert(UniCase::new(key), value);
    }
}

impl<'de> Deserialize<'de> for UsernameAliases {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = UsernameAliases;

            fn expecting(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
                fmt.write_str("a map")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut map = UsernameAliases::default();
                while let Some((key, value)) = access.next_entry()? {
                    map.insert(key, value);
                }
                Ok(map)
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomConfiguration {
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub simple_rooms: Vec<String>,
    pub secret: Option<String>,
}

pub struct RoomConfigurationRef<'a> {
    pub rooms: &'a [String],
    pub simple_rooms: &'a [String],
    pub secret: &'a str,
}

impl Config {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let server = Url::parse(&env::var("PSDEVBOT_SERVER")?)?;
        let user = env::var("PSDEVBOT_USER")?;
        let password = env::var("PSDEVBOT_PASSWORD")?;
        let secret = env::var("PSDEVBOT_SECRET")?;
        let port = match env::var("PSDEVBOT_PORT") {
            Ok(port) => port.parse()?,
            Err(_) => 3030,
        };
        let default_room_name = env::var("PSDEVBOT_ROOM").ok();
        let room_configuration = env::var("PSDEVBOT_PROJECT_CONFIGURATION")
            .map(|json| {
                serde_json::from_str(&json)
                    .expect("PSDEVBOT_PROJECT_CONFIGURATION should be valid JSON")
            })
            .ok();
        if default_room_name.is_none() && room_configuration.is_none() {
            panic!("At least one of PSDEVBOT_ROOM or PSDEVBOT_PROJECT_CONFIGURATION needs to be provided");
        }
        let github_api = env::var("PSDEVBOT_GITHUB_API_USER").ok().and_then(|user| {
            let password = env::var("PSDEVBOT_GITHUB_API_PASSWORD").ok()?;
            Some(Mutex::new(GitHubApi::new(user, password)))
        });
        let username_aliases = env::var("PSDEVBOT_USERNAME_ALIASES")
            .map(|json| {
                serde_json::from_str(&json).expect("PSDEVBOT_USERNAME_ALIASES should be valid JSON")
            })
            .unwrap_or_default();
        Ok(Self {
            server,
            user,
            password,
            secret,
            port,
            default_room_name,
            room_configuration: room_configuration.unwrap_or_default(),
            github_api,
            username_aliases,
        })
    }

    pub fn all_rooms(&self) -> HashSet<&str> {
        self.room_configuration
            .values()
            .flat_map(|r| r.rooms.iter().chain(&r.simple_rooms))
            .chain(&self.default_room_name)
            .map(String::as_str)
            .collect()
    }

    pub fn rooms_for(&self, name: &str) -> RoomConfigurationRef<'_> {
        if let Some(RoomConfiguration {
            rooms,
            simple_rooms,
            secret,
        }) = self.room_configuration.get(name)
        {
            RoomConfigurationRef {
                rooms,
                simple_rooms,
                secret: secret.as_deref().unwrap_or(&self.secret),
            }
        } else {
            RoomConfigurationRef {
                rooms: self
                    .default_room_name
                    .as_ref()
                    .map(slice::from_ref)
                    .unwrap_or_default(),
                simple_rooms: &[],
                secret: &self.secret,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Config, RoomConfiguration, UsernameAliases};
    use std::collections::HashMap;

    fn base_config() -> Config {
        Config {
            server: "wss://localhost/showdown/websocket".parse().unwrap(),
            user: "".into(),
            password: "".into(),
            secret: "".into(),
            port: 3030,
            default_room_name: None,
            room_configuration: HashMap::new(),
            github_api: None,
            username_aliases: UsernameAliases::default(),
        }
    }

    #[test]
    fn test_all_rooms_default_room() {
        let mut config = base_config();
        config.default_room_name = Some("room".into());
        let mut rooms: Vec<_> = config.all_rooms().into_iter().collect();
        rooms.sort_unstable();
        assert_eq!(rooms, ["room"]);
    }

    #[test]
    fn test_all_rooms_room_configuration() {
        let mut config = base_config();
        config.room_configuration.insert(
            "Project".into(),
            RoomConfiguration {
                rooms: vec!["a".into(), "b".into()],
                simple_rooms: vec![],
                secret: None,
            },
        );
        config.room_configuration.insert(
            "AnotherProject".into(),
            RoomConfiguration {
                rooms: vec!["b".into(), "c".into()],
                simple_rooms: vec![],
                secret: None,
            },
        );
        config.room_configuration.insert(
            "StupidProject".into(),
            RoomConfiguration {
                rooms: vec![],
                simple_rooms: vec!["d".into()],
                secret: None,
            },
        );
        let mut rooms: Vec<_> = config.all_rooms().into_iter().collect();
        rooms.sort_unstable();
        assert_eq!(rooms, ["a", "b", "c", "d"]);
    }

    #[test]
    fn test_username_aliases() {
        let mut username_aliases = UsernameAliases::default();
        username_aliases.insert("A".into(), "Awesome".into());
        assert_eq!(username_aliases.get("a"), "Awesome");
        assert_eq!(username_aliases.get("b"), "b");
    }
}
