use serde_json::{Value, json};

use super::*;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("the config is JSON")
}

#[test]
fn the_config_has_the_login_and_its_base64_auth() {
    let config = docker_config_json(
        "https://index.docker.io/v1/",
        "me",
        "p@ss",
        "me@example.test",
    );
    assert_eq!(
        parsed(&config),
        json!({"auths": {"https://index.docker.io/v1/": {
            "username": "me",
            "password": "p@ss",
            "email": "me@example.test",
            "auth": "bWU6cEBzcw==",
        }}})
    );
}

#[test]
fn an_empty_email_is_left_out() {
    let config = docker_config_json("registry.example.test", "me", "pw", "");
    let entry = &parsed(&config)["auths"]["registry.example.test"];
    assert!(entry.get("email").is_none());
    assert_eq!(entry["auth"], "bWU6cHc=");
}

#[test]
fn quotes_in_a_password_stay_valid_json() {
    let config = docker_config_json("r", "me", r#"a"b\c"#, "");
    assert_eq!(parsed(&config)["auths"]["r"]["password"], r#"a"b\c"#);
}
