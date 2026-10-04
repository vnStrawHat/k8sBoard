use super::*;

fn parsed(text: &str) -> String {
    ProxyUrl::parse(text).expect("a valid proxy URL").display()
}

#[test]
fn proxy_url_parse_table() {
    for (text, shown) in [
        ("http://p:3128", "http://p:3128"),
        ("socks5://[::1]:1080", "socks5://[::1]:1080"),
        ("HTTP://P:1/", "http://P:1"),
        ("SOCKS5://h:1", "socks5://h:1"),
        ("  http://proxy.example  ", "http://proxy.example"),
        ("http://p:65535", "http://p:65535"),
    ] {
        assert_eq!(parsed(text), shown, "{text}");
    }
    let too_long = format!("http://{}", "a".repeat(2_049));
    for (text, error) in [
        ("https://p", ProxyUrlError::Scheme),
        ("ftp://p", ProxyUrlError::Scheme),
        ("p:3128", ProxyUrlError::Scheme),
        ("http://u:p@p:1", ProxyUrlError::Credentials),
        ("http://u@p", ProxyUrlError::Credentials),
        ("socks5://:x@p", ProxyUrlError::Credentials),
        ("http://:1", ProxyUrlError::Host),
        ("http://", ProxyUrlError::Host),
        ("http://p:0", ProxyUrlError::Port),
        ("http://p:70000", ProxyUrlError::Port),
        ("http://p:", ProxyUrlError::Port),
        ("http://p:+5", ProxyUrlError::Port),
        ("http://p:x", ProxyUrlError::Port),
        ("http://p/x", ProxyUrlError::Path),
        ("http://p?q", ProxyUrlError::Path),
        ("http://p#f", ProxyUrlError::Path),
        ("http://p space", ProxyUrlError::Malformed),
        ("http://p\u{7}", ProxyUrlError::Malformed),
        ("", ProxyUrlError::Malformed),
        ("   ", ProxyUrlError::Malformed),
        ("http://[::1", ProxyUrlError::Malformed),
        (too_long.as_str(), ProxyUrlError::TooLong),
    ] {
        assert_eq!(ProxyUrl::parse(text), Err(error), "{text:?}");
    }
}

#[test]
fn a_multi_byte_start_is_a_scheme_error_and_never_a_panic() {
    for text in ["htté://p", "é", "ééééééééé", "socks5é://p"] {
        assert_eq!(ProxyUrl::parse(text), Err(ProxyUrlError::Scheme), "{text}");
    }
}

#[test]
fn proxy_url_display_is_scheme_host_port() {
    assert_eq!(parsed("HTTP://p:3128/"), "http://p:3128");
}

#[test]
fn a_parse_error_never_carries_the_text() {
    let error = ProxyUrl::parse("http://zed:hunter2@proxy:3128").expect_err("userinfo");
    let text = format!("{error} {error:?}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("zed"), "{text}");
}

#[test]
fn debug_of_proxy_types_shows_host_only() {
    let url = ProxyUrl::parse("socks5://proxy.example:1080").expect("valid");
    assert_eq!(
        format!("{url:?}"),
        "ProxyUrl(\"socks5://proxy.example:1080\")"
    );
    assert_eq!(
        format!("{:?}", ProxyChoice::Url(url)),
        "Url(ProxyUrl(\"socks5://proxy.example:1080\"))"
    );
    assert_eq!(format!("{:?}", ProxyChoice::Kubeconfig), "Kubeconfig");
    assert_eq!(format!("{:?}", ProxyChoice::Direct), "Direct");
}

fn uri(text: &str) -> http::Uri {
    text.parse().expect("a URI")
}

#[test]
fn proxy_choice_sets_the_config() {
    let loaded = Some(uri("http://u:p@kube-proxy:3128"));
    let custom = ProxyChoice::Url(ProxyUrl::parse("socks5://settings:1080").expect("valid"));
    // The kubeconfig sets a proxy-url.
    assert_eq!(
        ProxyChoice::Kubeconfig.config_uri(true, loaded.clone()),
        loaded
    );
    assert_eq!(ProxyChoice::Direct.config_uri(true, loaded.clone()), None);
    assert_eq!(
        custom.config_uri(true, loaded.clone()),
        Some(uri("socks5://settings:1080"))
    );
    // It does not: what kube loaded is HTTPS_PROXY, which is dropped.
    let ambient = Some(uri("http://ambient:8080"));
    assert_eq!(
        ProxyChoice::Kubeconfig.config_uri(false, ambient.clone()),
        None
    );
    assert_eq!(ProxyChoice::Direct.config_uri(false, ambient.clone()), None);
    assert_eq!(
        custom.config_uri(false, ambient),
        Some(uri("socks5://settings:1080"))
    );
    assert_eq!(ProxyChoice::Kubeconfig.config_uri(true, None), None);
}
