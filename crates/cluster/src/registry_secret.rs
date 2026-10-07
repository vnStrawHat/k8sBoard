//! The `.dockerconfigjson` of a docker-registry Secret: what `kubectl create secret docker-registry`
//! and `docker login` write. Pure; the text holds a password, so it is wiped on drop.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

/// `{"auths":{server:{username,password,email?,auth}}}`; `auth` is `username:password` in base64.
/// An empty `email` is left out.
pub fn docker_config_json(
    server: &str,
    username: &str,
    password: &str,
    email: &str,
) -> Zeroizing<String> {
    let mut entry = Map::new();
    entry.insert("username".to_owned(), json!(username));
    entry.insert("password".to_owned(), json!(password));
    if !email.is_empty() {
        entry.insert("email".to_owned(), json!(email));
    }
    let login = Zeroizing::new(format!("{username}:{password}"));
    entry.insert("auth".to_owned(), json!(STANDARD.encode(login.as_bytes())));
    let config = json!({ "auths": { server: Value::Object(entry) } });
    Zeroizing::new(config.to_string())
}

#[cfg(test)]
#[path = "registry_secret_tests.rs"]
mod registry_secret_tests;
