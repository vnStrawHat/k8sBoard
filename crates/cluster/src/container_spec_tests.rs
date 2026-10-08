use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    ConfigMapEnvSource, ConfigMapKeySelector, ConfigMapProjection, ConfigMapVolumeSource,
    DownwardAPIVolumeSource, EmptyDirVolumeSource, EnvVarSource, ExecAction, GRPCAction,
    HTTPGetAction, HTTPHeader, HostPathVolumeSource, NFSVolumeSource, ObjectFieldSelector,
    PersistentVolumeClaimVolumeSource, Pod, PodSpec, ProjectedVolumeSource, ResourceFieldSelector,
    ResourceRequirements, SecretEnvSource, SecretKeySelector, SecretProjection, SecretVolumeSource,
    TCPSocketAction, VolumeProjection,
};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::pod::pod_summary;

use super::*;

fn quantities(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, Quantity>> {
    Some(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Quantity((*value).to_owned())))
            .collect(),
    )
}

fn container_with_probe(probe: Probe) -> Container {
    Container {
        name: "main".to_owned(),
        readiness_probe: Some(probe),
        ..Default::default()
    }
}

fn readiness(container: &Container) -> ProbeSummary {
    container_probes(container)
        .readiness
        .expect("a readiness probe")
}

fn http_probe(path: Option<&str>) -> Probe {
    Probe {
        http_get: Some(HTTPGetAction {
            path: path.map(str::to_owned),
            port: IntOrString::Int(8080),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn env_var(name: &str, value: Option<&str>, from: Option<EnvVarSource>) -> EnvVar {
    EnvVar {
        name: name.to_owned(),
        value: value.map(str::to_owned),
        value_from: from,
    }
}

#[test]
fn resources_union_requests_and_limits_in_order() {
    let container = Container {
        resources: Some(ResourceRequirements {
            requests: quantities(&[("memory", "64Mi"), ("cpu", "100m"), ("zeta", "1")]),
            limits: quantities(&[
                ("ephemeral-storage", "1Gi"),
                ("memory", "128Mi"),
                ("alpha", "2"),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let listed: Vec<_> = container_resources(&container)
        .into_iter()
        .map(|resource| (resource.name, resource.request, resource.limit))
        .collect();
    let text = |value: &str| Some(value.to_owned());
    assert_eq!(
        listed,
        [
            ("cpu".to_owned(), text("100m"), None),
            ("memory".to_owned(), text("64Mi"), text("128Mi")),
            ("ephemeral-storage".to_owned(), None, text("1Gi")),
            ("alpha".to_owned(), None, text("2")),
            ("zeta".to_owned(), text("1"), None),
        ]
    );
    assert!(container_resources(&Container::default()).is_empty());
}

#[test]
fn probe_actions_map_each_handler() {
    let http = Probe {
        http_get: Some(HTTPGetAction {
            path: Some("/healthz".to_owned()),
            port: IntOrString::String("http".to_owned()),
            scheme: Some("HTTPS".to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        readiness(&container_with_probe(http)).action,
        ProbeAction::HttpGet {
            scheme: "HTTPS".to_owned(),
            port: "http".to_owned(),
            path: "/healthz".to_owned(),
        }
    );
    assert_eq!(
        readiness(&container_with_probe(http_probe(None))).action,
        ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/".to_owned(),
        }
    );

    let tcp = Probe {
        tcp_socket: Some(TCPSocketAction {
            port: IntOrString::Int(5432),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        readiness(&container_with_probe(tcp)).action,
        ProbeAction::TcpSocket {
            port: "5432".to_owned()
        }
    );

    let grpc = |port: i32| Probe {
        grpc: Some(GRPCAction {
            port,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        readiness(&container_with_probe(grpc(9000))).action,
        ProbeAction::Grpc { port: 9000 }
    );
    assert_eq!(
        readiness(&container_with_probe(grpc(70_000))).action,
        ProbeAction::Unknown
    );

    let exec = Probe {
        exec: Some(ExecAction::default()),
        ..Default::default()
    };
    assert_eq!(
        readiness(&container_with_probe(exec)).action,
        ProbeAction::Exec {
            command: Vec::new()
        }
    );
    assert_eq!(
        readiness(&container_with_probe(Probe::default())).action,
        ProbeAction::Unknown
    );
}

#[test]
fn probe_defaults_period_and_threshold() {
    let bare = readiness(&container_with_probe(Probe::default()));
    assert_eq!((bare.period_seconds, bare.failure_threshold), (10, 3));
    assert_eq!(bare.initial_delay_seconds, 0);

    let tuned = readiness(&container_with_probe(Probe {
        period_seconds: Some(5),
        failure_threshold: Some(7),
        initial_delay_seconds: Some(20),
        ..Default::default()
    }));
    assert_eq!((tuned.period_seconds, tuned.failure_threshold), (5, 7));
    assert_eq!(tuned.initial_delay_seconds, 20);
}

#[test]
fn probe_defaults_timeout_to_one_second() {
    let bare = readiness(&container_with_probe(Probe::default()));
    assert_eq!(bare.timeout_seconds, 1);

    let tuned = readiness(&container_with_probe(Probe {
        timeout_seconds: Some(4),
        ..Default::default()
    }));
    assert_eq!(tuned.timeout_seconds, 4);
}

#[test]
fn probes_are_read_per_kind() {
    let container = Container {
        liveness_probe: Some(Probe::default()),
        startup_probe: Some(Probe::default()),
        ..Default::default()
    };
    let probes = container_probes(&container);
    assert!(probes.liveness.is_some());
    assert!(probes.readiness.is_none());
    assert!(probes.startup.is_some());
}

#[test]
fn http_probe_path_drops_query_and_headers() {
    let mut probe = http_probe(Some("/ready?token=SECRETQ"));
    if let Some(http) = probe.http_get.as_mut() {
        http.host = Some("secret-host.internal".to_owned());
        http.http_headers = Some(vec![HTTPHeader {
            name: "Authorization".to_owned(),
            value: "Bearer SECRETHEADER".to_owned(),
        }]);
    }
    let summary = readiness(&container_with_probe(probe));
    assert_eq!(
        summary.action,
        ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/ready".to_owned(),
        }
    );
    let debug = format!("{summary:?}");
    for secret in ["SECRETQ", "SECRETHEADER", "Authorization", "secret-host"] {
        assert!(!debug.contains(secret), "{secret} leaked: {debug}");
    }
}

#[test]
fn exec_probe_keeps_its_argv() {
    let probe = Probe {
        exec: Some(ExecAction {
            command: Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "curl -u admin:SECRETEXEC".to_owned(),
            ]),
        }),
        ..Default::default()
    };
    let summary = readiness(&container_with_probe(probe));
    assert_eq!(
        summary.action,
        ProbeAction::Exec {
            command: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "curl -u admin:SECRETEXEC".to_owned()
            ]
        }
    );
}

#[test]
fn env_keeps_names_and_sources_never_values() {
    let config_map = EnvVarSource {
        config_map_key_ref: Some(ConfigMapKeySelector {
            name: "app-config".to_owned(),
            key: "mode".to_owned(),
            optional: None,
        }),
        ..Default::default()
    };
    let secret = EnvVarSource {
        secret_key_ref: Some(SecretKeySelector {
            name: "app-secret".to_owned(),
            key: "password".to_owned(),
            optional: None,
        }),
        ..Default::default()
    };
    let field = EnvVarSource {
        field_ref: Some(ObjectFieldSelector {
            field_path: "status.podIP".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let resource = EnvVarSource {
        resource_field_ref: Some(ResourceFieldSelector {
            resource: "limits.cpu".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let container = Container {
        name: "main".to_owned(),
        env: Some(vec![
            env_var("LITERAL", Some("SECRETLITERAL"), None),
            env_var("EMPTY", None, None),
            env_var("FROM_MAP", None, Some(config_map)),
            env_var("FROM_SECRET", None, Some(secret)),
            env_var("POD_IP", None, Some(field)),
            env_var("CPU_LIMIT", None, Some(resource)),
            env_var("ODD", None, Some(EnvVarSource::default())),
        ]),
        ..Default::default()
    };
    let listed: Vec<_> = env_entries(&container)
        .into_iter()
        .map(|entry| (entry.name, entry.source))
        .collect();
    assert_eq!(
        listed,
        [
            ("LITERAL".to_owned(), EnvSource::Literal),
            ("EMPTY".to_owned(), EnvSource::Literal),
            (
                "FROM_MAP".to_owned(),
                EnvSource::ConfigMapKey {
                    name: "app-config".to_owned(),
                    key: "mode".to_owned()
                }
            ),
            (
                "FROM_SECRET".to_owned(),
                EnvSource::SecretKey {
                    name: "app-secret".to_owned(),
                    key: "password".to_owned()
                }
            ),
            (
                "POD_IP".to_owned(),
                EnvSource::Field {
                    path: "status.podIP".to_owned()
                }
            ),
            (
                "CPU_LIMIT".to_owned(),
                EnvSource::ResourceField {
                    resource: "limits.cpu".to_owned()
                }
            ),
            ("ODD".to_owned(), EnvSource::Unknown),
        ]
    );

    let pod = Pod {
        spec: Some(PodSpec {
            containers: vec![container],
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!format!("{:?}", pod_summary(&pod)).contains("SECRETLITERAL"));
}

#[test]
fn env_from_keeps_source_and_prefix() {
    let container = Container {
        env_from: Some(vec![
            ApiEnvFromSource {
                config_map_ref: Some(ConfigMapEnvSource {
                    name: "app-config".to_owned(),
                    optional: None,
                }),
                prefix: Some("APP_".to_owned()),
                ..Default::default()
            },
            ApiEnvFromSource {
                secret_ref: Some(SecretEnvSource {
                    name: "app-secret".to_owned(),
                    optional: None,
                }),
                prefix: Some(String::new()),
                ..Default::default()
            },
            ApiEnvFromSource::default(),
        ]),
        ..Default::default()
    };
    assert_eq!(
        env_from_entries(&container),
        [
            EnvFromEntry {
                source: EnvFromSource::ConfigMap {
                    name: "app-config".to_owned()
                },
                prefix: Some("APP_".to_owned()),
            },
            EnvFromEntry {
                source: EnvFromSource::Secret {
                    name: "app-secret".to_owned()
                },
                prefix: None,
            },
            EnvFromEntry {
                source: EnvFromSource::Unknown,
                prefix: None,
            },
        ]
    );
}

fn volume(name: &str) -> Volume {
    Volume {
        name: name.to_owned(),
        ..Default::default()
    }
}

fn mount(name: &str, path: &str) -> VolumeMount {
    VolumeMount {
        name: name.to_owned(),
        mount_path: path.to_owned(),
        ..Default::default()
    }
}

#[test]
fn mounts_resolve_volume_sources() {
    let volumes = [
        Volume {
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_owned(),
                ..Default::default()
            }),
            ..volume("config")
        },
        Volume {
            secret: Some(SecretVolumeSource {
                secret_name: Some("app-secret".to_owned()),
                ..Default::default()
            }),
            ..volume("secret")
        },
        Volume {
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "data-claim".to_owned(),
                ..Default::default()
            }),
            ..volume("data")
        },
        Volume {
            empty_dir: Some(EmptyDirVolumeSource::default()),
            ..volume("scratch")
        },
        Volume {
            host_path: Some(HostPathVolumeSource {
                path: "/var/log".to_owned(),
                ..Default::default()
            }),
            ..volume("logs")
        },
        Volume {
            projected: Some(ProjectedVolumeSource::default()),
            ..volume("token")
        },
        Volume {
            downward_api: Some(DownwardAPIVolumeSource::default()),
            ..volume("labels")
        },
        Volume {
            nfs: Some(NFSVolumeSource::default()),
            ..volume("share")
        },
    ];
    let mut read_only = mount("config", "/etc/app");
    read_only.read_only = Some(true);
    let mut sub_path = mount("secret", "/etc/key");
    sub_path.sub_path = Some("key.pem".to_owned());
    let mut empty_sub_path = mount("data", "/data");
    empty_sub_path.sub_path = Some(String::new());
    let container = Container {
        volume_mounts: Some(vec![
            read_only,
            sub_path,
            empty_sub_path,
            mount("scratch", "/tmp"),
            mount("logs", "/logs"),
            mount("token", "/token"),
            mount("labels", "/labels"),
            mount("share", "/share"),
            mount("gone", "/gone"),
        ]),
        ..Default::default()
    };
    let listed: Vec<_> = mount_entries(&container, &volumes)
        .into_iter()
        .map(|entry| (entry.path, entry.source, entry.is_read_only, entry.sub_path))
        .collect();
    let named = |name: &str| name.to_owned();
    assert_eq!(
        listed,
        [
            (
                named("/etc/app"),
                VolumeSource::ConfigMap {
                    name: named("app-config")
                },
                true,
                None
            ),
            (
                named("/etc/key"),
                VolumeSource::Secret {
                    name: named("app-secret")
                },
                false,
                Some(named("key.pem"))
            ),
            (
                named("/data"),
                VolumeSource::PersistentVolumeClaim {
                    claim: named("data-claim")
                },
                false,
                None
            ),
            (named("/tmp"), VolumeSource::EmptyDir, false, None),
            (
                named("/logs"),
                VolumeSource::HostPath {
                    path: named("/var/log")
                },
                false,
                None
            ),
            (
                named("/token"),
                VolumeSource::Projected {
                    config_maps: Vec::new(),
                    secrets: Vec::new()
                },
                false,
                None
            ),
            (named("/labels"), VolumeSource::DownwardApi, false, None),
            (named("/share"), VolumeSource::Other, false, None),
            (named("/gone"), VolumeSource::Other, false, None),
        ]
    );
}

#[test]
fn image_digest_reads_after_last_at_or_bare_sha256() {
    let cases = [
        ("docker-pullable://nginx@sha256:ab", Some("sha256:ab")),
        ("registry/app@sha256:ab@sha256:cd", Some("sha256:cd")),
        ("sha256:cd", Some("sha256:cd")),
        ("docker://nginx", None),
        ("nginx@", None),
        ("", None),
    ];
    for (id, expected) in cases {
        assert_eq!(image_digest(id).as_deref(), expected, "{id}");
    }
}

#[test]
fn projected_volume_keeps_secret_names() {
    let projected = Volume {
        projected: Some(ProjectedVolumeSource {
            sources: Some(vec![
                VolumeProjection {
                    config_map: Some(ConfigMapProjection {
                        name: "kube-root-ca.crt".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                VolumeProjection {
                    secret: Some(SecretProjection {
                        name: "token-secret".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                VolumeProjection {
                    config_map: Some(ConfigMapProjection {
                        name: "extra".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..volume("bundle")
    };
    let container = Container {
        volume_mounts: Some(vec![mount("bundle", "/bundle")]),
        ..Default::default()
    };
    let entries = mount_entries(&container, &[projected]);
    assert_eq!(
        entries[0].source,
        VolumeSource::Projected {
            config_maps: vec!["kube-root-ca.crt".to_owned(), "extra".to_owned()],
            secrets: vec!["token-secret".to_owned()]
        }
    );
}
